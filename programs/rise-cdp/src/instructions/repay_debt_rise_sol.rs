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
#[inline(never)]
fn accrue_interest(
    position: &mut CdpPosition,
    cdp_config: &CdpConfig,
    collateral_config: &CollateralConfig,
    staking_supply: u128,
    current_slot: u64,
) -> Result<()> {
    if current_slot > position.last_accrual_slot && position.rise_sol_debt_principal > 0 {
        let slots_elapsed = current_slot
            .checked_sub(position.last_accrual_slot)
            .ok_or(CdpError::MathOverflow)? as u128;

        let ceiling = staking_supply
            .checked_mul(cdp_config.debt_ceiling_multiplier_bps as u128)
            .ok_or(CdpError::MathOverflow)?
            .checked_div(10_000)
            .ok_or(CdpError::MathOverflow)?;

        let utilization_bps: u128 = if ceiling == 0 {
            10_000
        } else {
            (cdp_config.cdp_rise_sol_minted
                .checked_mul(10_000)
                .ok_or(CdpError::MathOverflow)?
                .checked_div(ceiling)
                .ok_or(CdpError::MathOverflow)?)
                .min(10_000)
        };

        let optimal = collateral_config.optimal_utilization_bps as u128;

        let effective_rate_bps: u128 = if utilization_bps <= optimal {
            let slope1_contribution = if optimal == 0 {
                0
            } else {
                (collateral_config.rate_slope1_bps as u128)
                    .checked_mul(utilization_bps)
                    .ok_or(CdpError::MathOverflow)?
                    .checked_div(optimal)
                    .ok_or(CdpError::MathOverflow)?
            };
            (collateral_config.base_rate_bps as u128)
                .checked_add(slope1_contribution)
                .ok_or(CdpError::MathOverflow)?
        } else {
            let excess = utilization_bps
                .checked_sub(optimal)
                .ok_or(CdpError::MathOverflow)?;
            let range = 10_000u128
                .checked_sub(optimal)
                .ok_or(CdpError::MathOverflow)?;
            let slope2_contribution = if range == 0 {
                collateral_config.rate_slope2_bps as u128
            } else {
                (collateral_config.rate_slope2_bps as u128)
                    .checked_mul(excess)
                    .ok_or(CdpError::MathOverflow)?
                    .checked_div(range)
                    .ok_or(CdpError::MathOverflow)?
            };
            (collateral_config.base_rate_bps as u128)
                .checked_add(collateral_config.rate_slope1_bps as u128)
                .ok_or(CdpError::MathOverflow)?
                .checked_add(slope2_contribution)
                .ok_or(CdpError::MathOverflow)?
        };

        let interest = (position.rise_sol_debt_principal as u128)
            .checked_mul(effective_rate_bps)
            .ok_or(CdpError::MathOverflow)?
            .checked_mul(slots_elapsed)
            .ok_or(CdpError::MathOverflow)?
            .checked_div(10_000)
            .ok_or(CdpError::MathOverflow)?
            .checked_div(CollateralConfig::SLOTS_PER_YEAR)
            .ok_or(CdpError::MathOverflow)?;

        let interest_u64 = u64::try_from(interest).map_err(|_| CdpError::MathOverflow)?;

        if interest_u64 > 0 {
            position.interest_accrued = position
                .interest_accrued
                .checked_add(interest_u64)
                .ok_or(CdpError::MathOverflow)?;
            position.last_accrual_slot = current_slot;
        }
    }
    Ok(())
}

pub fn handler<'info>(
    ctx: Context<'_, '_, '_, 'info, RepayDebtRiseSol<'info>>,
    payment_rise_sol: u64,
    shortfall_route_plan_data: Vec<u8>,
    shortfall_quoted_out: u64,
    shortfall_slippage_bps: u16,
) -> Result<()> {
    require!(payment_rise_sol > 0, CdpError::ZeroAmount);

    // ── Interest accrual (extracted to avoid BPF stack overflow) ──────────────
    {
        let current_slot = Clock::get()?.slot;
        let staking_supply = ctx.accounts.global_pool.staking_rise_sol_supply;
        accrue_interest(
            &mut ctx.accounts.position,
            &ctx.accounts.cdp_config,
            &ctx.accounts.collateral_config,
            staking_supply,
            current_slot,
        )?;
    }

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
            // remaining_accounts order (all 14 required for shortfall path):
            //  [0]  collateral_mint
            //  [1]  sol_payment_config
            //  [2]  price_update (collateral Pyth feed)
            //  [3]  sol_price_update (SOL/USD Pyth feed)
            //  [4]  treasury
            //  [5]  reserve_vault
            //  [6]  pool_vault
            //  [7]  wsol_mint
            //  [8]  cdp_wsol_buyback_vault
            //  [9]  jupiter_program
            //  [10] jupiter_program_authority
            //  [11] jupiter_event_authority
            //  [12] shortfall_jupiter_source_token
            //  [13] shortfall_jupiter_destination_token
            // Clone AccountInfos from remaining_accounts in a separate scope so the
            // borrow of `ctx.remaining_accounts` is released before `ctx.accounts` is used.
            let (
                collateral_mint_ai, sol_payment_config_ai,
                price_update_ai, sol_price_update_ai,
                treasury_ai, reserve_vault_ai, pool_vault_ai,
                wsol_mint_ai, cdp_wsol_buyback_vault_ai,
                jupiter_program_ai, jupiter_authority_ai, jupiter_event_ai,
                jup_src_token_ai, jup_dst_token_ai,
            ) = {
                let rem = ctx.remaining_accounts;
                require!(rem.len() >= 14, CdpError::MissingShortfallAccounts);
                (
                    rem[0].clone(), rem[1].clone(),
                    rem[2].clone(), rem[3].clone(),
                    rem[4].clone(), rem[5].clone(), rem[6].clone(),
                    rem[7].clone(), rem[8].clone(),
                    rem[9].clone(), rem[10].clone(), rem[11].clone(),
                    rem[12].clone(), rem[13].clone(),
                )
            };

            // Validate the Jupiter program and buyback vault to prevent fund redirection.
            require_keys_eq!(jupiter_program_ai.key(), crate::jupiter::PROGRAM_ID, CdpError::InvalidAccount);
            let (expected_buyback_vault, _) = Pubkey::find_program_address(
                &[b"cdp_wsol_buyback_vault"],
                &crate::ID,
            );
            require_keys_eq!(cdp_wsol_buyback_vault_ai.key(), expected_buyback_vault, CdpError::InvalidAccount);

            // Read collateral mint decimals from raw SPL Mint data (decimals at byte 44).
            let decimals = {
                let data = collateral_mint_ai.try_borrow_data()?;
                require!(data.len() >= 45, CdpError::InvalidAccount);
                data[44]
            };

            // Deserialize sol_payment_config to get SOL/USD Pyth feed ID.
            let sol_feed_id = {
                use anchor_lang::AccountDeserialize;
                let data = sol_payment_config_ai.try_borrow_data()?;
                let cfg = PaymentConfig::try_deserialize(&mut &data[..])?;
                cfg.pyth_price_feed.to_bytes()
            };

            // Compute how much SOL the shortfall tokens are worth via Pyth.
            let coll_price = crate::pyth::get_pyth_price_info(
                &price_update_ai,
                &ctx.accounts.collateral_config.pyth_price_feed.to_bytes(),
            )?;
            let sol_price = crate::pyth::get_pyth_price_info(&sol_price_update_ai, &sol_feed_id)?;

            let dec_scale = 10u128.pow(decimals as u32);

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
                            treasury:               treasury_ai.clone(),
                            reserve_vault:          reserve_vault_ai.clone(),
                            cdp_wsol_buyback_vault: cdp_wsol_buyback_vault_ai.clone(),
                            system_program:         ctx.accounts.system_program.to_account_info(),
                        },
                        signer_seeds,
                    ),
                    shortfall_sol,
                )?;

                // Reflect the lamport deposit in the WSOL token account balance.
                token::sync_native(CpiContext::new(
                    ctx.accounts.token_program.to_account_info(),
                    SyncNative { account: cdp_wsol_buyback_vault_ai.clone() },
                ))?;

                // Jupiter swap: cdp_wsol_buyback_vault (WSOL) → borrower_collateral_account
                crate::jupiter::shared_accounts_route(
                    &jupiter_program_ai,
                    &jupiter_authority_ai,
                    &ctx.accounts.cdp_config.to_account_info(),
                    &cdp_wsol_buyback_vault_ai,
                    &jup_src_token_ai,
                    &jup_dst_token_ai,
                    &ctx.accounts.borrower_collateral_account.to_account_info(),
                    &wsol_mint_ai,
                    &collateral_mint_ai,
                    &jupiter_event_ai,
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
                            account:     cdp_wsol_buyback_vault_ai.clone(),
                            destination: pool_vault_ai.clone(),
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
    pub position: Box<Account<'info, CdpPosition>>,

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

    pub staking_program: Program<'info, RiseStaking>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,

    /// Global borrow rewards config — total_cdp_debt decremented on principal paydown.
    #[account(
        mut,
        seeds = [b"borrow_rewards_config"],
        bump = borrow_rewards_config.bump
    )]
    pub borrow_rewards_config: Box<Account<'info, BorrowRewardsConfig>>,

    /// Per-position borrow rewards — settled before debt is reduced.
    #[account(
        mut,
        seeds = [b"borrow_rewards", position.key().as_ref()],
        bump = borrow_rewards.bump,
        constraint = borrow_rewards.position == position.key()
    )]
    pub borrow_rewards: Box<Account<'info, BorrowRewards>>,
}
