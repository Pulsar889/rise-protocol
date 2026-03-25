use anchor_lang::prelude::*;
use crate::state::CollateralConfig;
use crate::errors::CdpError;

pub fn handler(
    ctx: Context<UpdateCollateralConfig>,
    max_ltv_bps: Option<u16>,
    liquidation_threshold_bps: Option<u16>,
    liquidation_penalty_bps: Option<u16>,
    base_rate_bps: Option<u32>,
    rate_slope1_bps: Option<u32>,
    rate_slope2_bps: Option<u32>,
    optimal_utilization_bps: Option<u16>,
    conversion_slippage_bps: Option<u16>,
    active: Option<bool>,
) -> Result<()> {
    let config = &mut ctx.accounts.collateral_config;

    if let Some(ltv) = max_ltv_bps {
        require!(ltv <= 10_000, CdpError::ZeroAmount);
        config.max_ltv_bps = ltv;
        msg!("Updated max LTV to {} bps", ltv);
    }

    if let Some(threshold) = liquidation_threshold_bps {
        require!(threshold <= 10_000, CdpError::ZeroAmount);
        config.liquidation_threshold_bps = threshold;
        msg!("Updated liquidation threshold to {} bps", threshold);
    }

    if let Some(penalty) = liquidation_penalty_bps {
        require!(penalty <= 10_000, CdpError::ZeroAmount);
        config.liquidation_penalty_bps = penalty;
        msg!("Updated liquidation penalty to {} bps", penalty);
    }

    if let Some(rate) = base_rate_bps {
        config.base_rate_bps = rate;
        msg!("Updated base rate to {} bps", rate);
    }

    if let Some(slope1) = rate_slope1_bps {
        config.rate_slope1_bps = slope1;
        msg!("Updated rate slope1 to {} bps", slope1);
    }

    if let Some(slope2) = rate_slope2_bps {
        config.rate_slope2_bps = slope2;
        msg!("Updated rate slope2 to {} bps", slope2);
    }

    if let Some(optimal) = optimal_utilization_bps {
        require!(optimal <= 10_000, CdpError::ZeroAmount);
        config.optimal_utilization_bps = optimal;
        msg!("Updated optimal utilization to {} bps", optimal);
    }

    if let Some(slippage) = conversion_slippage_bps {
        require!(slippage <= 10_000, CdpError::ZeroAmount);
        config.conversion_slippage_bps = slippage;
        msg!("Updated conversion slippage to {} bps", slippage);
    }

    if let Some(is_active) = active {
        config.active = is_active;
        msg!("Updated active status to {}", is_active);
    }

    msg!("Collateral config updated for mint: {}", config.mint);

    Ok(())
}

#[derive(Accounts)]
pub struct UpdateCollateralConfig<'info> {
    /// Protocol authority — only this account can update collateral configs.
    pub authority: Signer<'info>,

    /// The collateral config to update.
    #[account(
        mut,
        seeds = [b"collateral_config", collateral_config.mint.as_ref()],
        bump = collateral_config.bump
    )]
    pub collateral_config: Account<'info, CollateralConfig>,
}
