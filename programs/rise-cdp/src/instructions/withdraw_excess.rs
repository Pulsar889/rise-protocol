use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::state::{CdpPosition, CollateralConfig, CdpConfig, PaymentConfig};
use crate::errors::CdpError;
use rise_staking::state::GlobalPool;
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;

pub fn handler(ctx: Context<WithdrawExcess>, amount: u64) -> Result<()> {
    require!(ctx.accounts.position.is_open, CdpError::PositionClosed);
    require!(amount > 0, CdpError::ZeroAmount);

    // ── Inline interest accrual ────────────────────────────────────────────────
    {
        let current_slot = Clock::get()?.slot;
        let position = &mut ctx.accounts.position;
        if current_slot > position.last_accrual_slot && position.rise_sol_debt_principal > 0 {
            let slots_elapsed = current_slot
                .checked_sub(position.last_accrual_slot)
                .ok_or(CdpError::MathOverflow)? as u128;

            let cdp_config = &ctx.accounts.cdp_config;
            let config = &ctx.accounts.collateral_config;
            let staking_supply = ctx.accounts.global_pool.staking_rise_sol_supply;

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

            let optimal = config.optimal_utilization_bps as u128;

            let effective_rate_bps: u128 = if utilization_bps <= optimal {
                let slope1_contribution = if optimal == 0 {
                    0
                } else {
                    (config.rate_slope1_bps as u128)
                        .checked_mul(utilization_bps)
                        .ok_or(CdpError::MathOverflow)?
                        .checked_div(optimal)
                        .ok_or(CdpError::MathOverflow)?
                };
                (config.base_rate_bps as u128)
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
                    config.rate_slope2_bps as u128
                } else {
                    (config.rate_slope2_bps as u128)
                        .checked_mul(excess)
                        .ok_or(CdpError::MathOverflow)?
                        .checked_div(range)
                        .ok_or(CdpError::MathOverflow)?
                };
                (config.base_rate_bps as u128)
                    .checked_add(config.rate_slope1_bps as u128)
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
    }

    let position = &mut ctx.accounts.position;
    let config = &ctx.accounts.collateral_config;
    let liquidation_threshold_bps = config.liquidation_threshold_bps;

    // Get current prices
    let collateral_usd_price = crate::pyth::get_pyth_price(&ctx.accounts.price_update, &ctx.accounts.collateral_config.pyth_price_feed.to_bytes())?;
    let sol_usd_price = crate::pyth::get_pyth_price(&ctx.accounts.sol_price_update, &ctx.accounts.sol_payment_config.pyth_price_feed.to_bytes())?;

    let token_decimals = ctx.accounts.collateral_mint.decimals;
    let decimal_scale = 10u128.pow(token_decimals as u32);

    // Current collateral USD value
    let collateral_usd_value = (position.collateral_amount_original as u128)
        .checked_mul(collateral_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(decimal_scale)
        .ok_or(CdpError::MathOverflow)?;

    // Current debt USD value
    let total_owed = position.total_rise_sol_owed().ok_or(CdpError::MathOverflow)?;
    let debt_usd = (total_owed as u128)
        .checked_mul(sol_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(1_000_000_000)
        .ok_or(CdpError::MathOverflow)?;

    // Required collateral at safe withdrawal LTV (80% of max LTV)
    let safe_ltv_bps = (config.max_ltv_bps as u32)
        .checked_mul(80)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(100)
        .ok_or(CdpError::MathOverflow)? as u128;

    let required_collateral_usd = debt_usd
        .checked_mul(10_000)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(safe_ltv_bps)
        .ok_or(CdpError::MathOverflow)?;

    // Excess USD value
    require!(
        collateral_usd_value > required_collateral_usd,
        CdpError::InsufficientExcess
    );

    let excess_usd = collateral_usd_value
        .checked_sub(required_collateral_usd)
        .ok_or(CdpError::MathOverflow)?;

    // Convert requested amount to USD
    let requested_usd = (amount as u128)
        .checked_mul(collateral_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(decimal_scale)
        .ok_or(CdpError::MathOverflow)?;

    require!(requested_usd <= excess_usd, CdpError::InsufficientExcess);

    // Transfer excess collateral immediately to borrower
    let config_mint_ref = config.mint.as_ref();
    let seeds = &[
        b"collateral_vault".as_ref(),
        config_mint_ref,
        &[ctx.bumps.collateral_vault],
    ];
    let signer = &[&seeds[..]];

    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        Transfer {
            from: ctx.accounts.collateral_vault.to_account_info(),
            to: ctx.accounts.borrower_collateral_account.to_account_info(),
            authority: ctx.accounts.collateral_vault.to_account_info(),
        },
        signer,
    );
    token::transfer(cpi_ctx, amount)?;

    position.collateral_amount_original = position
        .collateral_amount_original
        .checked_sub(amount)
        .ok_or(CdpError::MathOverflow)?;

    ctx.accounts.collateral_config.total_collateral_entitlements = ctx
        .accounts
        .collateral_config
        .total_collateral_entitlements
        .saturating_sub(amount);

    // Update collateral USD value
    position.collateral_usd_value = (position.collateral_amount_original as u128)
        .checked_mul(collateral_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(decimal_scale)
        .ok_or(CdpError::MathOverflow)?;

    // Recompute health factor to reflect the reduced collateral
    position.health_factor = CdpPosition::compute_health_factor(
        position.collateral_usd_value,
        debt_usd,
        liquidation_threshold_bps,
    ).ok_or(CdpError::MathOverflow)?;

    msg!("Excess collateral withdrawn: {} tokens", amount);

    Ok(())
}


#[derive(Accounts)]
pub struct WithdrawExcess<'info> {
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
        constraint = collateral_config.mint == position.collateral_mint @ CdpError::CollateralNotAccepted
    )]
    pub collateral_config: Account<'info, CollateralConfig>,

    #[account(constraint = collateral_mint.key() == collateral_config.mint @ CdpError::CollateralNotAccepted)]
    pub collateral_mint: Account<'info, anchor_spl::token::Mint>,

    #[account(
        mut,
        constraint = borrower_collateral_account.mint == collateral_config.mint,
        constraint = borrower_collateral_account.owner == borrower.key()
    )]
    pub borrower_collateral_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"collateral_vault", collateral_config.mint.as_ref()],
        bump,
        constraint = collateral_vault.mint == collateral_config.mint
    )]
    pub collateral_vault: Account<'info, TokenAccount>,

    /// Global CDP config — needed for debt ceiling / utilization in interest accrual.
    #[account(
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Box<Account<'info, CdpConfig>>,

    /// GlobalPool from the staking program — read for staking supply in interest accrual.
    #[account(
        seeds = [b"global_pool"],
        seeds::program = rise_staking::ID,
        bump = global_pool.bump
    )]
    pub global_pool: Box<Account<'info, GlobalPool>>,

    /// SOL payment config — provides the registered SOL/USD price feed pubkey for validation.
    #[account(
        seeds = [b"payment_config", anchor_lang::solana_program::system_program::ID.as_ref()],
        bump = sol_payment_config.bump,
    )]
    pub sol_payment_config: Box<Account<'info, PaymentConfig>>,

    /// Pyth PriceUpdateV2 for collateral token — feed_id validated inside get_pyth_price.
    pub price_update: Account<'info, PriceUpdateV2>,

    /// Pyth PriceUpdateV2 for SOL/USD — feed_id validated inside get_pyth_price.
    pub sol_price_update: Account<'info, PriceUpdateV2>,

    pub token_program: Program<'info, Token>,
}
