use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, Burn, Mint};
use crate::state::{CdpPosition, CollateralConfig, CdpConfig, BorrowRewards, BorrowRewardsConfig};
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
/// Economic note on interest: when interest is paid in riseSOL, the fee revenue
/// accrues to all riseSOL holders implicitly (via reduced supply / higher backing
/// ratio) rather than flowing explicitly to cdp_fee_vault as it does in the
/// SOL/token repayment path. This is the natural trade-off of accepting riseSOL.
/// `shortfall_route_plan_data` / `shortfall_quoted_out` / `shortfall_slippage_bps` are used
/// only on full repayment when seized collateral must be bought back via Jupiter.
/// The SOL source for the buyback is `treasury_vault`, but transferring from it requires
/// a new `rise_staking::withdraw_treasury_for_cdp_buyback` instruction (not yet built).
/// Pass empty bytes / 0 for these params in normal (no-shortfall) usage.
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
        CdpError::RepaymentExceedsDebt
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

        // Keep borrow rewards debt tracker in sync.
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
    // Interest riseSOL is burned but doesn't reduce cdp_rise_sol_minted (interest
    // is not principal). Without this CPI, staking_rise_sol_supply would stay too
    // high, making the exchange rate too low and under-paying remaining stakers.
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

    // ── Full repayment: return collateral and close position ──────────────────
    let is_fully_repaid =
        position.interest_accrued == 0 && position.rise_sol_debt_principal == 0;

    if is_fully_repaid {
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

        // If collateral was previously seized for liquidity, buy it back using
        // SOL from treasury_vault. The riseSOL payment is burned (no payment SOL
        // exists to divert), so the protocol treasury covers the buyback cost.
        // This is an unlikely edge case — treasury absorbs the small cost.
        if shortfall > 0 {
            // Collateral was previously seized; the protocol owes the borrower `shortfall` tokens.
            // Buyback flow once `rise_staking::withdraw_treasury_for_cdp_buyback` is built:
            //   1. Compute shortfall_sol = shortfall * collateral_price / sol_price (via Pyth)
            //   2. CPI rise_staking::withdraw_treasury_for_cdp_buyback(shortfall_sol)
            //      → transfers shortfall_sol from treasury_vault to cdp_wsol_vault
            //   3. token::sync_native(cdp_wsol_vault) to reflect new lamports as WSOL balance
            //   4. Jupiter swap: cdp_wsol_vault (WSOL) → borrower_collateral_account
            //      using shortfall_route_plan / shortfall_quoted_out / shortfall_slippage_bps
            //   5. Transfer collateral tokens to borrower_collateral_account
            //
            // Params are already in the instruction signature; Jupiter accounts need to be
            // added to RepayDebtRiseSol once the treasury CPI is built.
            let _ = (&shortfall_route_plan_data, shortfall_quoted_out, shortfall_slippage_bps);
            msg!(
                "WARN: Collateral shortfall of {} tokens — treasury buyback pending rise_staking update",
                shortfall
            );
        }

        position.is_open = false;
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
    #[account(mut)]
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

    /// Global CDP config — tracks total CDP riseSOL minted.
    /// Also used as PDA signer for the notify_rise_sol_burned CPI.
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

    /// Protocol treasury vault — source of SOL for collateral buyback if shortfall occurs.
    /// CHECK: treasury PDA from staking program. Only used in Jupiter buyback path (TODO).
    #[account(
        mut,
        seeds = [b"treasury_vault"],
        seeds::program = rise_staking::ID,
        bump
    )]
    pub treasury_vault: UncheckedAccount<'info>,

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
