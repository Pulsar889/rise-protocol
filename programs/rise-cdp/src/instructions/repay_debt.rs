use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token::{self, Token, TokenAccount, Transfer as TokenTransfer, Mint, CloseAccount, SyncNative};
use crate::state::{CdpPosition, CollateralConfig, PaymentConfig, CdpConfig, BorrowRewards, BorrowRewardsConfig};
use crate::errors::CdpError;
use rise_staking::state::GlobalPool;

/// Repay all or part of a CDP position's debt.
///
/// Accepts payment in native SOL or any SPL token configured in PaymentConfig
/// (USDC, USDT, BTC, ETH). For SPL tokens, Jupiter v6 swaps the payment tokens → WSOL → SOL
/// and the actual swap output is used as the payment value (no oracle dependency for the swap).
///
/// Payment is split into:
///   - Interest portion  → cdp_fee_vault  (swept later by collect_cdp_fees)
///   - Principal portion → pool_vault     (increases pool backing; riseSOL rate rises)
///
/// Interest is cleared before principal (standard lending convention).
/// On full repayment the position is closed and collateral returned to the borrower.
/// If collateral was previously seized (via redeem_collateral_for_liquidity), a second Jupiter
/// swap uses `shortfall_route_plan_data` to buy back the owed tokens from `shortfall_sol`
/// diverted from principal. Pass empty bytes / 0 for shortfall params when no shortfall is expected.
pub fn handler(
    ctx: Context<RepayDebt>,
    payment_amount: u64,
    route_plan_data: Vec<u8>,
    quoted_out_amount: u64,
    slippage_bps: u16,
    shortfall_route_plan_data: Vec<u8>,
    shortfall_quoted_out: u64,
    shortfall_slippage_bps: u16,
) -> Result<()> {
    require!(payment_amount > 0, CdpError::ZeroAmount);

    let payment_config = &ctx.accounts.payment_config;
    require!(payment_config.active, CdpError::PaymentConfigInactive);

    let is_native_sol = payment_config.is_native_sol();

    // ── Compute payment value in SOL lamports ───────────────────────────────
    let payment_sol_lamports: u64 = if is_native_sol {
        // Native SOL — no swap needed.
        payment_amount
    } else {
        // SPL token: swap payment tokens → WSOL via Jupiter; use actual output as SOL value.
        // borrower is a tx signer — their signing authority propagates through the CPI.
        crate::jupiter::shared_accounts_route(
            &ctx.accounts.jupiter_program,
            &ctx.accounts.jupiter_program_authority,
            &ctx.accounts.borrower.to_account_info(),       // user_transfer_authority
            &ctx.accounts.borrower_payment_account
                .as_ref()
                .expect("SPL repayment requires borrower_payment_account")
                .to_account_info(),
            &ctx.accounts.jupiter_source_token,
            &ctx.accounts.jupiter_destination_token,
            &ctx.accounts.cdp_wsol_vault.to_account_info(),
            &ctx.accounts.payment_mint
                .as_ref()
                .expect("SPL repayment requires payment_mint")
                .to_account_info(),
            &ctx.accounts.wsol_mint.to_account_info(),
            &ctx.accounts.jupiter_event_authority,
            &ctx.accounts.token_program.to_account_info(),
            &route_plan_data,
            payment_amount,
            quoted_out_amount,
            slippage_bps,
            &[], // borrower is a real signer; no PDA seeds needed
        )?;

        ctx.accounts.cdp_wsol_vault.reload()?;
        let wsol_received = ctx.accounts.cdp_wsol_vault.amount;

        // Unwrap WSOL → native SOL into cdp_fee_vault (SOL is routed from there below)
        let cdp_config_bump = ctx.accounts.cdp_config.bump;
        let config_seeds = &[b"cdp_config".as_ref(), &[cdp_config_bump]];
        let config_signer = &[&config_seeds[..]];

        token::close_account(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                CloseAccount {
                    account:     ctx.accounts.cdp_wsol_vault.to_account_info(),
                    destination: ctx.accounts.cdp_fee_vault.to_account_info(),
                    authority:   ctx.accounts.cdp_config.to_account_info(),
                },
                config_signer,
            ),
        )?;

        msg!("Jupiter swap: {} tokens → {} lamports SOL", payment_amount, wsol_received);
        wsol_received
    };

    // ── Convert payment SOL → riseSOL units using current exchange rate ────────
    let exchange_rate = ctx.accounts.global_pool.exchange_rate;
    let rate_scale = GlobalPool::RATE_SCALE;

    let payment_rise_sol_u128 = (payment_sol_lamports as u128)
        .checked_mul(rate_scale)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(exchange_rate)
        .ok_or(CdpError::MathOverflow)?;
    let payment_rise_sol = u64::try_from(payment_rise_sol_u128).map_err(|_| CdpError::MathOverflow)?;

    require!(payment_rise_sol > 0, CdpError::ZeroAmount);

    let position = &mut ctx.accounts.position;

    // ── Settle borrow rewards before reducing debt ────────────────────────────
    {
        let reward_per_token = ctx.accounts.borrow_rewards_config.reward_per_token;
        let current_debt = position.rise_sol_debt_principal;
        ctx.accounts.borrow_rewards.settle(reward_per_token, current_debt)?;
    }

    // ── Compute total outstanding debt and cap repayment ────────────────────
    let total_owed = position.total_rise_sol_owed().ok_or(CdpError::MathOverflow)?;
    require!(total_owed > 0, CdpError::ZeroAmount);

    let cleared_rise_sol = payment_rise_sol.min(total_owed);

    // ── Clear interest first, then principal ────────────────────────────────
    let (interest_cleared_rise_sol, principal_cleared_rise_sol) =
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
        .checked_sub(interest_cleared_rise_sol)
        .ok_or(CdpError::MathOverflow)?;

    position.rise_sol_debt_principal = position
        .rise_sol_debt_principal
        .checked_sub(principal_cleared_rise_sol)
        .ok_or(CdpError::MathOverflow)?;

    // ── Convert cleared riseSOL back to SOL lamports for routing ───────────────
    let interest_sol = (interest_cleared_rise_sol as u128)
        .checked_mul(exchange_rate)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(rate_scale)
        .ok_or(CdpError::MathOverflow)? as u64;

    let cleared_sol = (cleared_rise_sol as u128)
        .checked_mul(exchange_rate)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(rate_scale)
        .ok_or(CdpError::MathOverflow)? as u64;

    let principal_sol = cleared_sol.saturating_sub(interest_sol);

    // ── Decrement global CDP minted counter by principal cleared ────────────
    if principal_cleared_rise_sol > 0 {
        let cdp_config = &mut ctx.accounts.cdp_config;
        cdp_config.cdp_rise_sol_minted = cdp_config
            .cdp_rise_sol_minted
            .saturating_sub(principal_cleared_rise_sol as u128);

        ctx.accounts.borrow_rewards_config.total_cdp_debt = ctx
            .accounts.borrow_rewards_config.total_cdp_debt
            .saturating_sub(principal_cleared_rise_sol);
    }

    // ── Re-sync per-position reward_debt ─────────────────────────────────────
    {
        let reward_per_token = ctx.accounts.borrow_rewards_config.reward_per_token;
        let new_principal = position.rise_sol_debt_principal;
        ctx.accounts.borrow_rewards.sync_debt(reward_per_token, new_principal)?;
    }

    // ── Pre-compute collateral shortfall before routing ───────────────────────
    // Done here so routing amounts can be adjusted before any SOL transfers.
    let is_fully_repaid =
        position.interest_accrued == 0 && position.rise_sol_debt_principal == 0;

    let (shortfall_tokens, available_collateral, shortfall_sol_divert) = if is_fully_repaid {
        let owed = position.collateral_amount_original;
        let available = ctx.accounts.collateral_vault.amount.min(owed);
        let sf_tokens = owed.saturating_sub(available);

        let sf_sol = if sf_tokens > 0 && !shortfall_route_plan_data.is_empty() {
            let coll_price = crate::pyth::get_pyth_price(&ctx.accounts.pyth_price_feed)?;
            let sol_price  = crate::pyth::get_pyth_price(&ctx.accounts.sol_price_feed)?;
            let decimals   = ctx.accounts.collateral_mint.decimals;
            let dec_scale  = 10u128.pow(decimals as u32);

            // shortfall_usd (micro-USD) = sf_tokens * coll_price / decimal_scale
            let sf_usd = (sf_tokens as u128)
                .checked_mul(coll_price).ok_or(CdpError::MathOverflow)?
                .checked_div(dec_scale).ok_or(CdpError::MathOverflow)?;

            // shortfall_sol (lamports) = sf_usd * 1e9 / sol_price
            let sf_sol_raw = sf_usd
                .checked_mul(1_000_000_000u128).ok_or(CdpError::MathOverflow)?
                .checked_div(sol_price).ok_or(CdpError::MathOverflow)? as u64;

            sf_sol_raw.min(principal_sol) // can't divert more than what's going to pool
        } else {
            0u64
        };

        (sf_tokens, available, sf_sol)
    } else {
        (0u64, 0u64, 0u64)
    };

    let principal_to_pool = principal_sol.saturating_sub(shortfall_sol_divert);

    // ── Route payment ───────────────────────────────────────────────────────
    if is_native_sol {
        // Borrower transfers SOL directly to each destination.
        if interest_sol > 0 {
            system_program::transfer(
                CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.borrower.to_account_info(),
                        to:   ctx.accounts.cdp_fee_vault.to_account_info(),
                    },
                ),
                interest_sol,
            )?;
        }
        if principal_to_pool > 0 {
            system_program::transfer(
                CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.borrower.to_account_info(),
                        to:   ctx.accounts.pool_vault.to_account_info(),
                    },
                ),
                principal_to_pool,
            )?;
        }
        if shortfall_sol_divert > 0 {
            // Transfer shortfall SOL directly into cdp_wsol_buyback_vault lamports.
            // sync_native (below) will update its WSOL balance to match.
            system_program::transfer(
                CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.borrower.to_account_info(),
                        to:   ctx.accounts.cdp_wsol_buyback_vault.to_account_info(),
                    },
                ),
                shortfall_sol_divert,
            )?;
        }
    } else {
        // SPL path: all SOL is already in cdp_fee_vault (from WSOL close above).
        // Route principal to pool_vault; shortfall to buyback vault; interest stays in fee vault.
        let fee_vault_bump = ctx.bumps.cdp_fee_vault;
        let fee_seeds = &[b"cdp_fee_vault".as_ref(), &[fee_vault_bump]];
        let fee_signer = &[&fee_seeds[..]];

        if principal_to_pool > 0 {
            system_program::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.cdp_fee_vault.to_account_info(),
                        to:   ctx.accounts.pool_vault.to_account_info(),
                    },
                    fee_signer,
                ),
                principal_to_pool,
            )?;
        }
        if shortfall_sol_divert > 0 {
            system_program::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.cdp_fee_vault.to_account_info(),
                        to:   ctx.accounts.cdp_wsol_buyback_vault.to_account_info(),
                    },
                    fee_signer,
                ),
                shortfall_sol_divert,
            )?;
        }
    }

    // ── Full repayment: return collateral, execute buyback if needed ──────────
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
        let vault_seeds = &[b"collateral_vault".as_ref(), config_mint_ref, &[vault_bump]];
        let vault_signer = &[&vault_seeds[..]];

        if available_collateral > 0 {
            token::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    TokenTransfer {
                        from:      ctx.accounts.collateral_vault.to_account_info(),
                        to:        ctx.accounts.borrower_collateral_account.to_account_info(),
                        authority: ctx.accounts.collateral_vault.to_account_info(),
                    },
                    vault_signer,
                ),
                available_collateral,
            )?;
        }

        if shortfall_tokens > 0 && shortfall_sol_divert > 0 {
            // Wrap the lamports sitting in cdp_wsol_buyback_vault as WSOL.
            token::sync_native(CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                SyncNative {
                    account: ctx.accounts.cdp_wsol_buyback_vault.to_account_info(),
                },
            ))?;

            // Jupiter swap: cdp_wsol_buyback_vault (WSOL) → borrower_collateral_account
            let cdp_config_bump = ctx.accounts.cdp_config.bump;
            let config_seeds = &[b"cdp_config".as_ref(), &[cdp_config_bump]];
            let config_signer = &[&config_seeds[..]];

            crate::jupiter::shared_accounts_route(
                &ctx.accounts.jupiter_program,
                &ctx.accounts.jupiter_program_authority,
                &ctx.accounts.cdp_config.to_account_info(),              // PDA authority over vault
                &ctx.accounts.cdp_wsol_buyback_vault.to_account_info(),  // source (WSOL)
                &ctx.accounts.shortfall_jupiter_source_token,
                &ctx.accounts.shortfall_jupiter_destination_token,
                &ctx.accounts.borrower_collateral_account.to_account_info(), // dest (collateral)
                &ctx.accounts.wsol_mint.to_account_info(),               // source mint
                &ctx.accounts.collateral_mint.to_account_info(),         // dest mint
                &ctx.accounts.jupiter_event_authority,
                &ctx.accounts.token_program.to_account_info(),
                &shortfall_route_plan_data,
                shortfall_sol_divert,
                shortfall_quoted_out,
                shortfall_slippage_bps,
                config_signer,
            )?;

            // Close the buyback vault to sweep any residual WSOL (slippage dust) to pool_vault.
            // Without this, small leftover amounts would accumulate permanently in the global vault.
            token::close_account(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    CloseAccount {
                        account:     ctx.accounts.cdp_wsol_buyback_vault.to_account_info(),
                        destination: ctx.accounts.pool_vault.to_account_info(),
                        authority:   ctx.accounts.cdp_config.to_account_info(),
                    },
                    config_signer,
                ),
            )?;

            msg!(
                "Shortfall buyback: {} lamports WSOL → collateral tokens for borrower",
                shortfall_sol_divert
            );
        } else if shortfall_tokens > 0 {
            return Err(error!(CdpError::CollateralShortfall));
        }

        msg!(
            "Position fully repaid and closed. Collateral returned: {} (shortfall: {})",
            available_collateral,
            shortfall_tokens
        );
    }

    msg!("riseSOL interest cleared:   {}", interest_cleared_rise_sol);
    msg!("riseSOL principal cleared:  {}", principal_cleared_rise_sol);
    msg!("SOL to fee vault:           {}", interest_sol);
    msg!("SOL to pool backing:        {}", principal_to_pool);

    Ok(())
}

#[derive(Accounts)]
pub struct RepayDebt<'info> {
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

    #[account(
        seeds = [b"payment_config", payment_config.mint.as_ref()],
        bump = payment_config.bump
    )]
    pub payment_config: Box<Account<'info, PaymentConfig>>,

    #[account(
        seeds = [b"global_pool"],
        seeds::program = rise_staking::ID,
        bump = global_pool.bump
    )]
    pub global_pool: Box<Account<'info, GlobalPool>>,

    /// Global CDP config — tracks total minted; authority for cdp_wsol_vault and cdp_wsol_buyback_vault.
    #[account(
        mut,
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Box<Account<'info, CdpConfig>>,

    /// CDP fee vault — receives interest portion; also buffers SPL-path SOL before routing.
    /// CHECK: PDA verified by seeds; holds native SOL.
    #[account(
        mut,
        seeds = [b"cdp_fee_vault"],
        bump
    )]
    pub cdp_fee_vault: UncheckedAccount<'info>,

    /// CHECK: Staking pool SOL vault — receives principal portion.
    #[account(
        mut,
        seeds = [b"pool_vault"],
        seeds::program = rise_staking::ID,
        bump
    )]
    pub pool_vault: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [b"collateral_vault", collateral_config.mint.as_ref()],
        bump,
        constraint = collateral_vault.mint == collateral_config.mint
    )]
    pub collateral_vault: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = borrower_collateral_account.mint == collateral_config.mint,
        constraint = borrower_collateral_account.owner == borrower.key()
    )]
    pub borrower_collateral_account: Box<Account<'info, TokenAccount>>,

    /// Collateral token mint — needed for decimal scaling in shortfall SOL computation.
    #[account(constraint = collateral_mint.key() == collateral_config.mint @ CdpError::CollateralNotAccepted)]
    pub collateral_mint: Box<Account<'info, Mint>>,

    /// SOL payment config — provides the registered SOL/USD price feed pubkey for validation.
    #[account(
        seeds = [b"payment_config", anchor_lang::solana_program::system_program::ID.as_ref()],
        bump = sol_payment_config.bump,
    )]
    pub sol_payment_config: Box<Account<'info, PaymentConfig>>,

    /// CHECK: Pyth price feed for the collateral token — used only when shortfall buyback occurs.
    #[account(constraint = pyth_price_feed.key() == collateral_config.pyth_price_feed @ CdpError::WrongPriceFeed)]
    pub pyth_price_feed: AccountInfo<'info>,

    /// CHECK: Pyth price feed for SOL/USD — used only when shortfall buyback occurs.
    #[account(constraint = sol_price_feed.key() == sol_payment_config.pyth_price_feed @ CdpError::WrongPriceFeed)]
    pub sol_price_feed: AccountInfo<'info>,

    // ── SPL payment token accounts (pass None for native SOL) ────────────────

    pub payment_mint: Option<Box<Account<'info, Mint>>>,

    /// Borrower's payment token account — Jupiter's source for the inbound swap.
    #[account(mut)]
    pub borrower_payment_account: Option<Box<Account<'info, TokenAccount>>>,

    // ── WSOL / Jupiter accounts ───────────────────────────────────────────────

    /// Native SOL (WSOL) mint.
    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Box<Account<'info, Mint>>,

    /// Protocol WSOL buffer: receives Jupiter's inbound WSOL output, then closed to unwrap.
    /// Pre-initialized by init_wsol_vaults at deploy time.
    #[account(
        mut,
        seeds = [b"cdp_wsol_vault"],
        bump,
    )]
    pub cdp_wsol_vault: Box<Account<'info, TokenAccount>>,

    /// Protocol WSOL buyback vault: holds WSOL for the reverse swap (WSOL → collateral).
    /// Pre-initialized by init_wsol_vaults at deploy time.
    #[account(
        mut,
        seeds = [b"cdp_wsol_buyback_vault"],
        bump,
    )]
    pub cdp_wsol_buyback_vault: Box<Account<'info, TokenAccount>>,

    /// CHECK: Jupiter v6 program.
    #[account(address = crate::jupiter::PROGRAM_ID)]
    pub jupiter_program: AccountInfo<'info>,

    /// CHECK: Jupiter's shared authority PDA.
    pub jupiter_program_authority: AccountInfo<'info>,

    /// CHECK: Jupiter's event authority PDA.
    pub jupiter_event_authority: AccountInfo<'info>,

    /// CHECK: Jupiter's shared source token account for the inbound swap route.
    #[account(mut)]
    pub jupiter_source_token: AccountInfo<'info>,

    /// CHECK: Jupiter's shared destination token account for the inbound swap route.
    #[account(mut)]
    pub jupiter_destination_token: AccountInfo<'info>,

    /// CHECK: Jupiter's shared source token account for the shortfall buyback route (WSOL side).
    #[account(mut)]
    pub shortfall_jupiter_source_token: AccountInfo<'info>,

    /// CHECK: Jupiter's shared destination token account for the shortfall buyback route (collateral side).
    #[account(mut)]
    pub shortfall_jupiter_destination_token: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,

    #[account(
        mut,
        seeds = [b"borrow_rewards_config"],
        bump = borrow_rewards_config.bump
    )]
    pub borrow_rewards_config: Box<Account<'info, BorrowRewardsConfig>>,

    #[account(
        mut,
        seeds = [b"borrow_rewards", position.key().as_ref()],
        bump = borrow_rewards.bump,
        constraint = borrow_rewards.position == position.key()
    )]
    pub borrow_rewards: Box<Account<'info, BorrowRewards>>,
}
