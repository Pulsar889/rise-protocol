use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, Burn, Mint, SyncNative, CloseAccount};
use crate::state::{CdpPosition, CollateralConfig, CdpConfig, BorrowRewards, BorrowRewardsConfig, PaymentConfig};
use crate::errors::CdpError;
use rise_staking::program::RiseStaking;

/// Repay all or part of a CDP position's debt using riseSOL tokens directly.
///
/// The debt is already denominated in riseSOL units, so payment is 1:1 — no
/// price feed or exchange rate conversion is needed. The borrower's riseSOL
/// is burned, reducing the total riseSOL supply. The SOL that was backing
/// the burned riseSOL remains in the pool, so the exchange rate rises for all
/// remaining riseSOL holders.
///
/// Interest is cleared before principal (standard lending convention).
/// On full repayment the position is closed and collateral returned.
///
/// Shortfall buyback: if collateral was previously seized (via redeem_collateral_for_liquidity),
/// the protocol treasury funds the buyback. The staking program transfers `shortfall_sol` from
/// reserve_vault → cdp_wsol_buyback_vault, which is then wrapped as WSOL and swapped via
/// Jupiter → collateral tokens → borrower. Pass empty bytes / 0 for shortfall params in
/// the common case (no shortfall expected).
pub fn handler(
    ctx: Context<RepayDebtRiseSol>,
    payment_rise_sol: u64,
    shortfall_route_plan_data: Vec<u8>,
    shortfall_quoted_out: u64,
    shortfall_slippage_bps: u16,
) -> Result<()> {
    require!(payment_rise_sol > 0, CdpError::ZeroAmount);

    let position = &mut ctx.accounts.position;

    // ── Settle borrow rewards before reducing debt ────────────────────────────
    {
        let reward_per_token = ctx.accounts.borrow_rewards_config.reward_per_token;
        let current_debt = position.rise_sol_debt_principal;
        ctx.accounts.borrow_rewards.settle(reward_per_token, current_debt)?;
    }

    // ── Cap repayment at total debt ──────────────────────────────────────────
    let total_owed = position.total_rise_sol_owed().ok_or(CdpError::MathOverflow)?;
    require!(total_owed > 0, CdpError::ZeroAmount);

    let cleared_rise_sol = payment_rise_sol.min(total_owed);

    // ── Verify borrower holds enough riseSOL ────────────────────────────────────
    require!(
        ctx.accounts.borrower_rise_sol_account.amount >= cleared_rise_sol,
        CdpError::InsufficientRepaymentBalance
    );

    // ── Clear interest first, then principal ─────────────────────────────────
    let (interest_cleared, principal_cleared) =
        if cleared_rise_sol <= position.interest_accrued {
            (cleared_rise_sol, 0u64)
        } else {
            let remaining = cleared_rise_sol
                .checked_sub(position.interest_accrued)
                .ok_or(CdpError::MathOverflow)?;
            (position.interest_accrued, remaining)
        };

    position.interest_accrued = position
        .interest_accrued
        .checked_sub(interest_cleared)
        .ok_or(CdpError::MathOverflow)?;

    position.rise_sol_debt_principal = position
        .rise_sol_debt_principal
        .checked_sub(principal_cleared)
        .ok_or(CdpError::MathOverflow)?;

    // ── Decrement global CDP minted counter by principal cleared ────────────
    if principal_cleared > 0 {
        let cdp_config = &mut ctx.accounts.cdp_config;
        cdp_config.cdp_rise_sol_minted = cdp_config
            .cdp_rise_sol_minted
            .saturating_sub(principal_cleared as u128);

        ctx.accounts.borrow_rewards_config.total_cdp_debt = ctx
            .accounts.borrow_rewards_config.total_cdp_debt
            .saturating_sub(principal_cleared);
    }

    // ── Re-sync per-position reward_debt to new principal ────────────────────
    {
        let reward_per_token = ctx.accounts.borrow_rewards_config.reward_per_token;
        let new_principal = position.rise_sol_debt_principal;
        ctx.accounts.borrow_rewards.sync_debt(reward_per_token, new_principal)?;
    }

    // ── Burn cleared riseSOL from borrower ──────────────────────────────────────
    let cpi_ctx = CpiContext::new(
        ctx.accounts.token_program.to_account_info(),
        Burn {
            mint: ctx.accounts.rise_sol_mint.to_account_info(),
            from: ctx.accounts.borrower_rise_sol_account.to_account_info(),
            authority: ctx.accounts.borrower.to_account_info(),
        },
    );
    token::burn(cpi_ctx, cleared_rise_sol)?;

    // ── Notify staking pool of interest burn so exchange rate adjusts ────────────
    if interest_cleared > 0 {
        let bump = ctx.accounts.cdp_config.bump;
        let signer_seeds: &[&[&[u8]]] = &[&[b"cdp_config", &[bump]]];

        rise_staking::cpi::notify_rise_sol_burned(
            CpiContext::new_with_signer(
                ctx.accounts.staking_program.to_account_info(),
                rise_staking::cpi::accounts::NotifyRiseSolBurned {
                    cdp_config: ctx.accounts.cdp_config.to_account_info(),
                    global_pool: ctx.accounts.global_pool.to_account_info(),
                },
                signer_seeds,
            ),
            interest_cleared,
        )?;
    }

    // ── Full repayment: return collateral and execute treasury buyback if needed ──
    let is_fully_repaid =
        position.interest_accrued == 0 && position.rise_sol_debt_principal == 0;

    if is_fully_repaid {
        // Guard against reentrancy through collateral-return and Jupiter buyback CPIs.
        position.is_open = false;

        ctx.accounts.collateral_config.total_collateral_entitlements = ctx
            .accounts
            .collateral_config
            .total_collateral_entitlements
            .saturating_sub(position.collateral_amount_original);

        let collateral_config = &ctx.accounts.collateral_config;
        let config_mint_ref = collateral_config.mint.as_ref();
        let vault_bump = ctx.bumps.collateral_vault;
        let seeds = &[b"collateral_vault".as_ref(), config_mint_ref, &[vault_bump]];
        let signer = &[&seeds[..]];

        let owed = position.collateral_amount_original;
        let available = ctx.accounts.collateral_vault.amount.min(owed);
        let shortfall = owed.saturating_sub(available);

        // Transfer whatever collateral is in the vault
        if available > 0 {
            let cpi_ctx = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.collateral_vault.to_account_info(),
                    to: ctx.accounts.borrower_collateral_account.to_account_info(),
                    authority: ctx.accounts.collateral_vault.to_account_info(),
                },
                signer,
            );
            token::transfer(cpi_ctx, available)?;
        }

        if shortfall > 0 && !shortfall_route_plan_data.is_empty() {
            // Compute how much SOL the shortfall tokens are worth via Pyth.
            let coll_price = crate::pyth::get_pyth_price(&ctx.accounts.pyth_price_feed)?;
            let sol_price  = crate::pyth::get_pyth_price(&ctx.accounts.sol_price_feed)?;
            let decimals   = ctx.accounts.collateral_mint.decimals;
            let dec_scale  = 10u128.pow(decimals as u32);

            let sf_usd = (shortfall as u128)
                .checked_mul(coll_price).ok_or(CdpError::MathOverflow)?
                .checked_div(dec_scale).ok_or(CdpError::MathOverflow)?;

            let shortfall_sol = sf_usd
                .checked_mul(1_000_000_000u128).ok_or(CdpError::MathOverflow)?
                .checked_div(sol_price).ok_or(CdpError::MathOverflow)? as u64;

            if shortfall_sol > 0 {
                let bump = ctx.accounts.cdp_config.bump;
                let signer_seeds: &[&[&[u8]]] = &[&[b"cdp_config", &[bump]]];

                // CPI: reserve_vault → cdp_wsol_buyback_vault (native SOL transfer)
                rise_staking::cpi::withdraw_treasury_for_cdp_buyback(
                    CpiContext::new_with_signer(
                        ctx.accounts.staking_program.to_account_info(),
                        rise_staking::cpi::accounts::WithdrawTreasuryForCdpBuyback {
                            cdp_config:             ctx.accounts.cdp_config.to_account_info(),
                            global_pool:            ctx.accounts.global_pool.to_account_info(),
                            treasury:               ctx.accounts.treasury.to_account_info(),
                            reserve_vault:          ctx.accounts.reserve_vault.to_account_info(),
                            cdp_wsol_buyback_vault: ctx.accounts.cdp_wsol_buyback_vault.to_account_info(),
                            system_program:         ctx.accounts.system_program.to_account_info(),
                        },
                        signer_seeds,
                    ),
                    shortfall_sol,
                )?;

                // Reflect the lamport deposit in the WSOL token account balance.
                token::sync_native(CpiContext::new(
                    ctx.accounts.token_program.to_account_info(),
                    SyncNative {
                        account: ctx.accounts.cdp_wsol_buyback_vault.to_account_info(),
                    },
                ))?;

                // Jupiter swap: cdp_wsol_buyback_vault (WSOL) → borrower_collateral_account
                crate::jupiter::shared_accounts_route(
                    &ctx.accounts.jupiter_program,
                    &ctx.accounts.jupiter_program_authority,
                    &ctx.accounts.cdp_config.to_account_info(),              // PDA authority over vault
                    &ctx.accounts.cdp_wsol_buyback_vault.to_account_info(),  // source (WSOL)
                    &ctx.accounts.shortfall_jupiter_source_token,
                    &ctx.accounts.shortfall_jupiter_destination_token,
                    &ctx.accounts.borrower_collateral_account.to_account_info(), // dest
                    &ctx.accounts.wsol_mint.to_account_info(),               // source mint
                    &ctx.accounts.collateral_mint.to_account_info(),         // dest mint
                    &ctx.accounts.jupiter_event_authority,
                    &ctx.accounts.token_program.to_account_info(),
                    &shortfall_route_plan_data,
                    shortfall_sol,
                    shortfall_quoted_out,
                    shortfall_slippage_bps,
                    signer_seeds,
                )?;

                // Close the buyback vault to sweep any residual WSOL to pool_vault.
                token::close_account(
                    CpiContext::new_with_signer(
                        ctx.accounts.token_program.to_account_info(),
                        CloseAccount {
                            account:     ctx.accounts.cdp_wsol_buyback_vault.to_account_info(),
                            destination: ctx.accounts.pool_vault.to_account_info(),
                            authority:   ctx.accounts.cdp_config.to_account_info(),
                        },
                        signer_seeds,
                    ),
                )?;

                msg!(
                    "Treasury buyback complete: {} lamports WSOL → collateral tokens for borrower",
                    shortfall_sol
                );
            }
        } else if shortfall > 0 {
            msg!(
                "WARN: Collateral shortfall of {} tokens — no route plan provided, skipping buyback",
                shortfall
            );
        }

        msg!(
            "Position fully repaid (riseSOL) and closed. Collateral returned: {} (shortfall: {})",
            available,
            shortfall
        );
    }

    msg!("riseSOL interest cleared:  {}", interest_cleared);
    msg!("riseSOL principal cleared: {}", principal_cleared);
    msg!("riseSOL burned:            {}", cleared_rise_sol);

    Ok(())
}

#[derive(Accounts)]
pub struct RepayDebtRiseSol<'info> {
    #[account(mut)]
    pub borrower: Signer<'info>,

    #[account(
        mut,
        seeds = [b"cdp_position", borrower.key().as_ref(), &[position.nonce]],
        bump = position.bump,
        constraint = position.owner == borrower.key(),
        constraint = position.is_open @ CdpError::PositionClosed
    )]
    pub position: Account<'info, CdpPosition>,

    #[account(
        mut,
        seeds = [b"collateral_config", collateral_config.mint.as_ref()],
        bump = collateral_config.bump,
        constraint = collateral_config.mint == position.collateral_mint
    )]
    pub collateral_config: Box<Account<'info, CollateralConfig>>,

    /// The riseSOL mint — needed to burn tokens.
    #[account(
        mut,
        address = global_pool.rise_sol_mint
    )]
    pub rise_sol_mint: Box<Account<'info, Mint>>,

    /// Borrower's riseSOL token account to burn from.
    #[account(
        mut,
        constraint = borrower_rise_sol_account.mint == rise_sol_mint.key(),
        constraint = borrower_rise_sol_account.owner == borrower.key()
    )]
    pub borrower_rise_sol_account: Box<Account<'info, TokenAccount>>,

    /// Protocol collateral vault — returns tokens on full repayment.
    #[account(
        mut,
        seeds = [b"collateral_vault", collateral_config.mint.as_ref()],
        bump,
        constraint = collateral_vault.mint == collateral_config.mint
    )]
    pub collateral_vault: Box<Account<'info, TokenAccount>>,

    /// Borrower's collateral account — receives collateral back on full repayment.
    #[account(
        mut,
        constraint = borrower_collateral_account.mint == collateral_config.mint,
        constraint = borrower_collateral_account.owner == borrower.key()
    )]
    pub borrower_collateral_account: Box<Account<'info, TokenAccount>>,

    /// Collateral token mint — needed for decimal scaling in shortfall SOL computation.
    #[account(constraint = collateral_mint.key() == collateral_config.mint @ CdpError::CollateralNotAccepted)]
    pub collateral_mint: Box<Account<'info, Mint>>,

    /// Global CDP config — tracks total CDP riseSOL minted; PDA signer for staking CPIs.
    #[account(
        mut,
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Box<Account<'info, CdpConfig>>,

    /// GlobalPool from staking — updated by notify_rise_sol_burned CPI.
    #[account(
        mut,
        seeds = [b"global_pool"],
        seeds::program = rise_staking::ID,
        bump = global_pool.bump
    )]
    pub global_pool: Box<Account<'info, rise_staking::state::GlobalPool>>,

    /// Protocol treasury — reserve_lamports decremented by buyback withdrawal.
    #[account(
        mut,
        seeds = [b"protocol_treasury"],
        seeds::program = rise_staking::ID,
        bump = treasury.bump
    )]
    pub treasury: Box<Account<'info, rise_staking::state::ProtocolTreasury>>,

    /// Protocol reserve vault — source of buyback funds (shortfall path only).
    /// CHECK: PDA verified by seeds on the staking program.
    #[account(
        mut,
        seeds = [b"reserve_vault"],
        seeds::program = rise_staking::ID,
        bump
    )]
    pub reserve_vault: UncheckedAccount<'info>,

    /// Staking pool SOL vault — receives residual WSOL swept from buyback vault after swap.
    /// CHECK: PDA verified by seeds on the staking program.
    #[account(
        mut,
        seeds = [b"pool_vault"],
        seeds::program = rise_staking::ID,
        bump
    )]
    pub pool_vault: UncheckedAccount<'info>,

    /// Native SOL (WSOL) mint — needed for Jupiter buyback swap.
    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Box<Account<'info, Mint>>,

    /// Protocol WSOL buyback vault: receives treasury SOL (via staking CPI), wrapped as WSOL,
    /// then swapped → collateral tokens → borrower. Pre-initialized by init_wsol_vaults.
    #[account(
        mut,
        seeds = [b"cdp_wsol_buyback_vault"],
        bump,
        constraint = cdp_wsol_buyback_vault.mint == wsol_mint.key(),
        constraint = cdp_wsol_buyback_vault.owner == cdp_config.key(),
    )]
    pub cdp_wsol_buyback_vault: Box<Account<'info, TokenAccount>>,

    /// SOL payment config — provides the registered SOL/USD price feed pubkey for validation.
    #[account(
        seeds = [b"payment_config", anchor_lang::solana_program::system_program::ID.as_ref()],
        bump = sol_payment_config.bump,
    )]
    pub sol_payment_config: Box<Account<'info, PaymentConfig>>,

    /// CHECK: Pyth price feed for the collateral token (shortfall path only).
    #[account(constraint = pyth_price_feed.key() == collateral_config.pyth_price_feed @ CdpError::WrongPriceFeed)]
    pub pyth_price_feed: AccountInfo<'info>,

    /// CHECK: Pyth price feed for SOL/USD (shortfall path only).
    #[account(constraint = sol_price_feed.key() == sol_payment_config.pyth_price_feed @ CdpError::WrongPriceFeed)]
    pub sol_price_feed: AccountInfo<'info>,

    // ── Jupiter accounts (shortfall buyback path only) ────────────────────────

    /// CHECK: Jupiter v6 program.
    #[account(address = crate::jupiter::PROGRAM_ID)]
    pub jupiter_program: AccountInfo<'info>,

    /// CHECK: Jupiter's shared authority PDA.
    pub jupiter_program_authority: AccountInfo<'info>,

    /// CHECK: Jupiter's event authority PDA.
    pub jupiter_event_authority: AccountInfo<'info>,

    /// CHECK: Jupiter's shared source token account for the buyback route (WSOL side).
    #[account(mut)]
    pub shortfall_jupiter_source_token: AccountInfo<'info>,

    /// CHECK: Jupiter's shared destination token account for the buyback route (collateral side).
    #[account(mut)]
    pub shortfall_jupiter_destination_token: AccountInfo<'info>,

    pub staking_program: Program<'info, RiseStaking>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,

    /// Global borrow rewards config — total_cdp_debt decremented on principal paydown.
    #[account(
        mut,
        seeds = [b"borrow_rewards_config"],
        bump = borrow_rewards_config.bump
    )]
    pub borrow_rewards_config: Account<'info, BorrowRewardsConfig>,

    /// Per-position borrow rewards — settled before debt is reduced.
    #[account(
        mut,
        seeds = [b"borrow_rewards", position.key().as_ref()],
        bump = borrow_rewards.bump,
        constraint = borrow_rewards.position == position.key()
    )]
    pub borrow_rewards: Account<'info, BorrowRewards>,
}
