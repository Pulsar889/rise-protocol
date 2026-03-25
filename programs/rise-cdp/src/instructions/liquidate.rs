use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, Mint};
use crate::state::{CdpPosition, CollateralConfig, CdpConfig};
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
///   4. Remaining (debt-worth) collateral stays in vault — protocol owned
///      TODO: Jupiter CPI converts to SOL →
///        principal SOL → pool_vault  (maintains riseSOL backing)
///        interest SOL  → cdp_fee_vault (collected as fees via collect_cdp_fees)
///   5. Debt cancelled, position closed
///
/// The riseSOL that was minted stays in circulation — it is backed by the
/// converted SOL added to pool_vault, so the exchange rate is unaffected.
pub fn handler(ctx: Context<Liquidate>) -> Result<()> {
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

    // ── SOL targets for when Jupiter is integrated ───────────────────────────
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

    // ── Execute transfers ────────────────────────────────────────────────────
    let config_mint_ref = config.mint.as_ref();
    let vault_bump = ctx.bumps.collateral_vault;
    let seeds = &[b"collateral_vault".as_ref(), config_mint_ref, &[vault_bump]];
    let signer = &[&seeds[..]];

    // Trigger fee → caller
    if trigger_fee_tokens > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.collateral_vault.to_account_info(),
                    to: ctx.accounts.caller_collateral_account.to_account_info(),
                    authority: ctx.accounts.collateral_vault.to_account_info(),
                },
                signer,
            ),
            trigger_fee_tokens,
        )?;
    }

    // Excess → borrower
    if excess_tokens > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.collateral_vault.to_account_info(),
                    to: ctx.accounts.borrower_collateral_account.to_account_info(),
                    authority: ctx.accounts.collateral_vault.to_account_info(),
                },
                signer,
            ),
            excess_tokens,
        )?;
    }

    // Remaining collateral (debt worth) stays in collateral_vault — protocol owned.
    // TODO: Replace with Jupiter CPI that swaps remaining tokens → SOL, then:
    //   principal_sol_target ({} lamports) → pool_vault
    //   interest_sol_target  ({} lamports) → cdp_fee_vault
    msg!("TODO: Jupiter swap remaining collateral → SOL");
    msg!("  principal target: {} lamports → pool_vault", principal_sol_target);
    msg!("  interest  target: {} lamports → cdp_fee_vault", interest_sol_target);

    // ── Decrement entitlement counter — position is fully settled ────────────
    ctx.accounts.collateral_config.total_collateral_entitlements = ctx
        .accounts
        .collateral_config
        .total_collateral_entitlements
        .saturating_sub(position.collateral_amount_original);

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
    msg!("Trigger fee to caller:      {} tokens", trigger_fee_tokens);
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
    /// Receives the trigger fee as incentive.
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

    /// GlobalPool from staking — read for exchange rate to compute SOL targets.
    pub global_pool: Box<Account<'info, GlobalPool>>,

    /// Global CDP config — tracks total CDP riseSOL minted.
    #[account(
        mut,
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Box<Account<'info, CdpConfig>>,

    /// CHECK: Pyth price feed for the collateral token.
    pub pyth_price_feed: AccountInfo<'info>,

    /// CHECK: Pyth price feed for SOL/USD.
    pub sol_price_feed: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}
