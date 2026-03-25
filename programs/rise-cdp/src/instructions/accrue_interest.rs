use anchor_lang::prelude::*;
use crate::state::{CdpPosition, CollateralConfig, CdpConfig};
use crate::errors::CdpError;
use rise_staking::state::GlobalPool;

pub fn handler(ctx: Context<AccrueInterest>) -> Result<()> {
    let position = &mut ctx.accounts.position;
    let config = &ctx.accounts.collateral_config;

    require!(position.is_open, CdpError::PositionClosed);

    let current_slot = Clock::get()?.slot;

    if current_slot <= position.last_accrual_slot {
        return Ok(());
    }

    let slots_elapsed = current_slot
        .checked_sub(position.last_accrual_slot)
        .ok_or(CdpError::MathOverflow)? as u128;

    // ── Compute global utilization ────────────────────────────────────────────
    // utilization_bps = cdp_rise_sol_minted * 10_000 / ceiling
    // ceiling = staking_rise_sol_supply * debt_ceiling_multiplier_bps / 10_000
    let cdp_config = &ctx.accounts.cdp_config;
    let staking_supply = ctx.accounts.global_pool.staking_rise_sol_supply;

    let ceiling = staking_supply
        .checked_mul(cdp_config.debt_ceiling_multiplier_bps as u128)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(10_000)
        .ok_or(CdpError::MathOverflow)?;

    let utilization_bps: u128 = if ceiling == 0 {
        10_000 // treat as fully utilized if no ceiling set
    } else {
        (cdp_config.cdp_rise_sol_minted
            .checked_mul(10_000)
            .ok_or(CdpError::MathOverflow)?
            .checked_div(ceiling)
            .ok_or(CdpError::MathOverflow)?)
            .min(10_000)
    };

    // ── Kinked interest rate ──────────────────────────────────────────────────
    // Below kink: rate = base_rate + (utilization / optimal) * slope1
    // Above kink: rate = base_rate + slope1 + ((utilization - optimal) / (10_000 - optimal)) * slope2
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

    // ── Accrue interest ───────────────────────────────────────────────────────
    // interest = principal * effective_rate_bps * slots_elapsed / 10_000 / SLOTS_PER_YEAR
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

    position.interest_accrued = position
        .interest_accrued
        .checked_add(interest_u64)
        .ok_or(CdpError::MathOverflow)?;

    position.last_accrual_slot = current_slot;

    msg!("Utilization: {} bps", utilization_bps);
    msg!("Effective rate: {} bps annual", effective_rate_bps);
    msg!("Interest accrued: {} riseSOL units", interest_u64);
    msg!("Total interest:   {} riseSOL units", position.interest_accrued);

    Ok(())
}

#[derive(Accounts)]
pub struct AccrueInterest<'info> {
    /// Anyone can call this crank.
    pub caller: Signer<'info>,

    #[account(
        mut,
        constraint = position.is_open @ CdpError::PositionClosed
    )]
    pub position: Account<'info, CdpPosition>,

    #[account(
        seeds = [b"collateral_config", position.collateral_mint.as_ref()],
        bump = collateral_config.bump
    )]
    pub collateral_config: Account<'info, CollateralConfig>,

    /// Global CDP config — read for current cdp_rise_sol_minted and ceiling multiplier.
    #[account(
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Account<'info, CdpConfig>,

    /// GlobalPool from staking — read for staking_rise_sol_supply (ceiling denominator).
    pub global_pool: Account<'info, GlobalPool>,
}
