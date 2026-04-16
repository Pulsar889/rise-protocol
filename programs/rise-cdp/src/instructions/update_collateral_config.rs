use anchor_lang::prelude::*;
use crate::state::CollateralConfig;
use crate::errors::CdpError;

pub fn handler(
    ctx: Context<UpdateCollateralConfig>,
    feed_id: Option<Pubkey>,
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

    if let Some(fid) = feed_id {
        msg!("pyth feed_id: {} → {}", config.pyth_price_feed, fid);
        config.pyth_price_feed = fid;
    }

    if let Some(ltv) = max_ltv_bps {
        require!(ltv <= 9_500, CdpError::ZeroAmount);
        msg!("max_ltv_bps: {} → {}", config.max_ltv_bps, ltv);
        config.max_ltv_bps = ltv;
    }

    if let Some(threshold) = liquidation_threshold_bps {
        require!(threshold <= 9_800, CdpError::ZeroAmount);
        msg!("liquidation_threshold_bps: {} → {}", config.liquidation_threshold_bps, threshold);
        config.liquidation_threshold_bps = threshold;
    }

    if let Some(penalty) = liquidation_penalty_bps {
        require!(penalty <= 2_000, CdpError::ZeroAmount);
        msg!("liquidation_penalty_bps: {} → {}", config.liquidation_penalty_bps, penalty);
        config.liquidation_penalty_bps = penalty;
    }

    require!(
        config.liquidation_threshold_bps > config.max_ltv_bps,
        CdpError::ZeroAmount
    );

    if let Some(rate) = base_rate_bps {
        require!(rate <= 10_000, CdpError::ZeroAmount);
        msg!("base_rate_bps: {} → {}", config.base_rate_bps, rate);
        config.base_rate_bps = rate;
    }

    if let Some(slope1) = rate_slope1_bps {
        require!(slope1 <= 20_000, CdpError::ZeroAmount);
        msg!("rate_slope1_bps: {} → {}", config.rate_slope1_bps, slope1);
        config.rate_slope1_bps = slope1;
    }

    if let Some(slope2) = rate_slope2_bps {
        require!(slope2 <= 50_000, CdpError::ZeroAmount);
        msg!("rate_slope2_bps: {} → {}", config.rate_slope2_bps, slope2);
        config.rate_slope2_bps = slope2;
    }

    if let Some(optimal) = optimal_utilization_bps {
        require!(optimal <= 10_000, CdpError::ZeroAmount);
        msg!("optimal_utilization_bps: {} → {}", config.optimal_utilization_bps, optimal);
        config.optimal_utilization_bps = optimal;
    }

    if let Some(slippage) = conversion_slippage_bps {
        require!(slippage <= 10_000, CdpError::ZeroAmount);
        msg!("conversion_slippage_bps: {} → {}", config.conversion_slippage_bps, slippage);
        config.conversion_slippage_bps = slippage;
    }

    if let Some(is_active) = active {
        msg!("active: {} → {}", config.active, is_active);
        config.active = is_active;
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
