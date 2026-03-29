use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, Mint, CloseAccount};
use crate::state::{CdpPosition, CollateralConfig, CdpConfig, BorrowRewards, BorrowRewardsConfig};
use crate::errors::CdpError;
use rise_staking::state::GlobalPool;

/// Protocol-owned liquidation. Permissionless — any caller can trigger this on
/// an unhealthy position, but the program enforces validity. Proceeds go to the
/// protocol, not the caller, except for a trigger fee to incentivize bots.
///
/// Flow:
///   1. Verify health factor < 1.0 (program rejects invalid liquidations)
///   2. Caller receives liquidation_penalty_bps % of collateral (trigger fee)
///   3. Excess collateral above (debt + fee) returned to borrower
///   4. Jupiter CPI converts remaining debt-worth collateral → WSOL → SOL:
///        principal SOL → pool_vault  (maintains riseSOL backing)
///        interest SOL  → cdp_fee_vault (collected as fees via collect_cdp_fees)
///   5. Debt cancelled, position closed
pub fn handler(
    ctx: Context<Liquidate>,
    route_plan_data: Vec<u8>,
    quoted_out_amount: u64,
    slippage_bps: u16,
) -> Result<()> {
    let position = &mut ctx.accounts.position;
    let config = &ctx.accounts.collateral_config;

    require!(position.is_open, CdpError::PositionClosed);

    // ── Price feeds ──────────────────────────────────────────────────────────
    let collateral_usd_price = get_mock_price(&ctx.accounts.pyth_price_feed)?;
    let sol_usd_price = get_mock_price(&ctx.accounts.sol_price_feed)?;

    let token_decimals = ctx.accounts.collateral_mint.decimals;
    let decimal_scale = 10u128.pow(token_decimals as u32);

    // ── Collateral and debt USD values ───────────────────────────────────────
    let collateral_usd = (position.collateral_amount_original as u128)
        .checked_mul(collateral_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(decimal_scale)
        .ok_or(CdpError::MathOverflow)?;

    let total_owed = position.total_rise_sol_owed().ok_or(CdpError::MathOverflow)?;
    let debt_usd = (total_owed as u128)
        .checked_mul(sol_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(1_000_000_000)
        .ok_or(CdpError::MathOverflow)?;

    // ── Health check — program rejects if position is still healthy ──────────
    let health_factor = CdpPosition::compute_health_factor(
        collateral_usd,
        debt_usd,
        config.liquidation_threshold_bps,
    ).ok_or(CdpError::MathOverflow)?;

    require!(
        health_factor < CollateralConfig::RATE_SCALE,
        CdpError::PositionHealthy
    );

    // ── Trigger fee → caller (liquidation_penalty_bps % of collateral) ───────
    let trigger_fee_usd = collateral_usd
        .checked_mul(config.liquidation_penalty_bps as u128)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(10_000)
        .ok_or(CdpError::MathOverflow)?;

    let trigger_fee_tokens = trigger_fee_usd
        .checked_mul(decimal_scale)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(collateral_usd_price)
        .ok_or(CdpError::MathOverflow)? as u64;

    // ── Excess collateral → borrower (above debt + trigger fee) ─────────────
    let total_deducted_usd = debt_usd
        .checked_add(trigger_fee_usd)
        .ok_or(CdpError::MathOverflow)?;

    let excess_usd = collateral_usd.saturating_sub(total_deducted_usd);

    let excess_tokens = excess_usd
        .checked_mul(decimal_scale)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(collateral_usd_price)
        .ok_or(CdpError::MathOverflow)? as u64;

    // ── Expected SOL splits (proportioning of actual Jupiter output) ──────────
    let exchange_rate = ctx.accounts.global_pool.exchange_rate;
    let rate_scale = GlobalPool::RATE_SCALE;

    let principal_sol_target = (position.rise_sol_debt_principal as u128)
        .checked_mul(exchange_rate)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(rate_scale)
        .ok_or(CdpError::MathOverflow)? as u64;

    let interest_sol_target = (position.interest_accrued as u128)
        .checked_mul(exchange_rate)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(rate_scale)
        .ok_or(CdpError::MathOverflow)? as u64;

    // ── Vault signer seeds ───────────────────────────────────────────────────
    let config_mint_ref = config.mint.as_ref();
    let vault_bump = ctx.bumps.collateral_vault;
    let vault_seeds = &[b"collateral_vault".as_ref(), config_mint_ref, &[vault_bump]];
    let vault_signer = &[&vault_seeds[..]];

    // ── Trigger fee → caller ─────────────────────────────────────────────────
    if trigger_fee_tokens > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from:      ctx.accounts.collateral_vault.to_account_info(),
                    to:        ctx.accounts.caller_collateral_account.to_account_info(),
                    authority: ctx.accounts.collateral_vault.to_account_info(),
                },
                vault_signer,
            ),
            trigger_fee_tokens,
        )?;
    }

    // ── Excess → borrower ────────────────────────────────────────────────────
    if excess_tokens > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from:      ctx.accounts.collateral_vault.to_account_info(),
                    to:        ctx.accounts.borrower_collateral_account.to_account_info(),
                    authority: ctx.accounts.collateral_vault.to_account_info(),
                },
                vault_signer,
            ),
            excess_tokens,
        )?;
    }

    // ── Remaining debt-worth collateral → Jupiter → WSOL → SOL ──────────────
    let debt_worth_tokens = position.collateral_amount_original
        .saturating_sub(trigger_fee_tokens)
        .saturating_sub(excess_tokens);

    if debt_worth_tokens > 0 {
        // Jupiter v6 CPI: collateral_vault → cdp_wsol_vault (WSOL)
        // collateral_vault is both the source and the user_transfer_authority.
        crate::jupiter::shared_accounts_route(
            &ctx.accounts.jupiter_program,
            &ctx.accounts.jupiter_program_authority,
            &ctx.accounts.collateral_vault.to_account_info(),
            &ctx.accounts.collateral_vault.to_account_info(),
            &ctx.accounts.jupiter_source_token,
            &ctx.accounts.jupiter_destination_token,
            &ctx.accounts.cdp_wsol_vault.to_account_info(),
            &ctx.accounts.collateral_mint.to_account_info(),
            &ctx.accounts.wsol_mint.to_account_info(),
            &ctx.accounts.jupiter_event_authority,
            &ctx.accounts.token_program.to_account_info(),
            &route_plan_data,
            debt_worth_tokens,
            quoted_out_amount,
            slippage_bps,
            vault_signer,
        )?;

        // Record actual WSOL received
        ctx.accounts.cdp_wsol_vault.reload()?;
        let actual_sol = ctx.accounts.cdp_wsol_vault.amount;

        // Proportionally split between interest and principal
        let total_target = principal_sol_target.saturating_add(interest_sol_target);
        let actual_interest_sol = if total_target > 0 && actual_sol > 0 {
            (actual_sol as u128)
                .checked_mul(interest_sol_target as u128)
                .unwrap_or(0)
                .checked_div(total_target as u128)
                .unwrap_or(0) as u64
        } else {
            0
        };
        let actual_principal_sol = actual_sol.saturating_sub(actual_interest_sol);

        // Unwrap WSOL → native SOL: close cdp_wsol_vault → cdp_fee_vault
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

        // Route principal SOL from cdp_fee_vault → pool_vault
        if actual_principal_sol > 0 {
            let fee_vault_bump = ctx.bumps.cdp_fee_vault;
            let fee_seeds = &[b"cdp_fee_vault".as_ref(), &[fee_vault_bump]];
            let fee_signer = &[&fee_seeds[..]];

            system_program::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.cdp_fee_vault.to_account_info(),
                        to:   ctx.accounts.pool_vault.to_account_info(),
                    },
                    fee_signer,
                ),
                actual_principal_sol,
            )?;
        }
        // actual_interest_sol (+ rent) remains in cdp_fee_vault

        msg!("Jupiter swap: {} tokens → {} lamports SOL", debt_worth_tokens, actual_sol);
        msg!("  principal SOL → pool_vault:    {}", actual_principal_sol);
        msg!("  interest  SOL → cdp_fee_vault: {}", actual_interest_sol);
    }

    // ── Decrement entitlement counter ────────────────────────────────────────
    ctx.accounts.collateral_config.total_collateral_entitlements = ctx
        .accounts
        .collateral_config
        .total_collateral_entitlements
        .saturating_sub(position.collateral_amount_original);

    // ── Settle borrow rewards and update global debt tracker ─────────────────
    {
        let reward_per_token = ctx.accounts.borrow_rewards_config.reward_per_token;
        let current_debt = position.rise_sol_debt_principal;
        ctx.accounts.borrow_rewards.settle(reward_per_token, current_debt)?;

        ctx.accounts.borrow_rewards_config.total_cdp_debt = ctx
            .accounts.borrow_rewards_config.total_cdp_debt
            .saturating_sub(current_debt);
    }

    // ── Decrement global CDP minted counter ──────────────────────────────────
    let cdp_config = &mut ctx.accounts.cdp_config;
    cdp_config.cdp_rise_sol_minted = cdp_config
        .cdp_rise_sol_minted
        .saturating_sub(position.rise_sol_debt_principal as u128);

    // ── Cancel debt and close position ───────────────────────────────────────
    position.is_open = false;
    position.rise_sol_debt_principal = 0;
    position.interest_accrued = 0;

    msg!("Position liquidated — health factor was: {}", health_factor);
    msg!("Trigger fee to caller:       {} tokens", trigger_fee_tokens);
    msg!("Excess returned to borrower: {} tokens", excess_tokens);

    Ok(())
}

fn get_mock_price(price_feed: &AccountInfo) -> Result<u128> {
    let lamports = price_feed.lamports();
    require!(lamports > 0, CdpError::InvalidOraclePrice);
    Ok(lamports as u128)
}

#[derive(Accounts)]
pub struct Liquidate<'info> {
    /// Permissionless — any caller can trigger a valid liquidation.
    /// Receives the trigger fee; pays rent for cdp_wsol_vault if first use.
    #[account(mut)]
    pub caller: Signer<'info>,

    #[account(
        mut,
        constraint = position.is_open @ CdpError::PositionClosed
    )]
    pub position: Box<Account<'info, CdpPosition>>,

    #[account(
        mut,
        seeds = [b"collateral_config", collateral_config.mint.as_ref()],
        bump = collateral_config.bump
    )]
    pub collateral_config: Box<Account<'info, CollateralConfig>>,

    pub collateral_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [b"collateral_vault", collateral_config.mint.as_ref()],
        bump,
        constraint = collateral_vault.mint == collateral_config.mint
    )]
    pub collateral_vault: Box<Account<'info, TokenAccount>>,

    /// Caller's collateral token account — receives the trigger fee.
    #[account(
        mut,
        constraint = caller_collateral_account.mint == collateral_config.mint,
        constraint = caller_collateral_account.owner == caller.key()
    )]
    pub caller_collateral_account: Box<Account<'info, TokenAccount>>,

    /// Borrower's collateral account — receives excess collateral if any.
    #[account(
        mut,
        constraint = borrower_collateral_account.mint == collateral_config.mint
    )]
    pub borrower_collateral_account: Box<Account<'info, TokenAccount>>,

    /// GlobalPool from staking — read for exchange rate.
    pub global_pool: Box<Account<'info, GlobalPool>>,

    /// Global CDP config — authority for cdp_wsol_vault; tracks total minted.
    #[account(
        mut,
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Box<Account<'info, CdpConfig>>,

    /// CDP fee vault — receives interest portion of Jupiter swap output.
    /// CHECK: PDA verified by seeds; holds native SOL.
    #[account(
        mut,
        seeds = [b"cdp_fee_vault"],
        bump
    )]
    pub cdp_fee_vault: UncheckedAccount<'info>,

    /// CHECK: Staking pool SOL vault — receives principal portion of Jupiter swap output.
    #[account(mut)]
    pub pool_vault: UncheckedAccount<'info>,

    /// Native SOL (WSOL) mint — Jupiter outputs WSOL which is then unwrapped.
    pub wsol_mint: Box<Account<'info, Mint>>,

    /// Protocol WSOL buffer: receives Jupiter's WSOL output, then closed to unwrap.
    #[account(
        init_if_needed,
        payer = caller,
        token::mint = wsol_mint,
        token::authority = cdp_config,
        seeds = [b"cdp_wsol_vault"],
        bump,
    )]
    pub cdp_wsol_vault: Box<Account<'info, TokenAccount>>,

    /// CHECK: Pyth price feed for the collateral token.
    pub pyth_price_feed: AccountInfo<'info>,

    /// CHECK: Pyth price feed for SOL/USD.
    pub sol_price_feed: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,

    // ── Jupiter v6 accounts ──────────────────────────────────────────────────

    /// CHECK: Jupiter v6 program.
    #[account(address = crate::jupiter::PROGRAM_ID)]
    pub jupiter_program: AccountInfo<'info>,

    /// CHECK: Jupiter's shared authority PDA.
    pub jupiter_program_authority: AccountInfo<'info>,

    /// CHECK: Jupiter's event authority PDA.
    pub jupiter_event_authority: AccountInfo<'info>,

    /// CHECK: Jupiter's shared source token account for this route (from Jupiter quote API).
    #[account(mut)]
    pub jupiter_source_token: AccountInfo<'info>,

    /// CHECK: Jupiter's shared destination token account for this route (from Jupiter quote API).
    #[account(mut)]
    pub jupiter_destination_token: AccountInfo<'info>,

    // ── Borrow rewards ───────────────────────────────────────────────────────

    #[account(
        mut,
        seeds = [b"borrow_rewards_config"],
        bump = borrow_rewards_config.bump
    )]
    pub borrow_rewards_config: Account<'info, BorrowRewardsConfig>,

    #[account(
        mut,
        seeds = [b"borrow_rewards", position.key().as_ref()],
        bump = borrow_rewards.bump,
        close = caller
    )]
    pub borrow_rewards: Account<'info, BorrowRewards>,
}
