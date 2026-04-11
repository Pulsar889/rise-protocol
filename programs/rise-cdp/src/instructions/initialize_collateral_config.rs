use anchor_lang::prelude::*;
use anchor_spl::token::Mint;
use crate::state::CollateralConfig;
use crate::errors::CdpError;

pub fn handler(
    ctx: Context<InitializeCollateralConfig>,
    max_ltv_bps: u16,
    liquidation_threshold_bps: u16,
    liquidation_penalty_bps: u16,
    base_rate_bps: u32,
    rate_slope1_bps: u32,
    rate_slope2_bps: u32,
    optimal_utilization_bps: u16,
    conversion_slippage_bps: u16,
) -> Result<()> {
    require!(max_ltv_bps <= 9_500, CdpError::ZeroAmount);
    require!(liquidation_threshold_bps <= 9_800, CdpError::ZeroAmount);
    require!(liquidation_threshold_bps > max_ltv_bps, CdpError::ZeroAmount);
    require!(liquidation_penalty_bps <= 2_000, CdpError::ZeroAmount);
    require!(optimal_utilization_bps > 0 && optimal_utilization_bps < 10_000, CdpError::ZeroAmount);
    require!(base_rate_bps <= 10_000, CdpError::ZeroAmount);
    require!(rate_slope1_bps <= 20_000, CdpError::ZeroAmount);
    require!(rate_slope2_bps <= 50_000, CdpError::ZeroAmount);

    let config = &mut ctx.accounts.collateral_config;

    config.mint = ctx.accounts.collateral_mint.key();
    config.pyth_price_feed = ctx.accounts.pyth_price_feed.key();
    config.max_ltv_bps = max_ltv_bps;
    config.liquidation_threshold_bps = liquidation_threshold_bps;
    config.liquidation_penalty_bps = liquidation_penalty_bps;
    config.base_rate_bps = base_rate_bps;
    config.rate_slope1_bps = rate_slope1_bps;
    config.rate_slope2_bps = rate_slope2_bps;
    config.optimal_utilization_bps = optimal_utilization_bps;
    config.conversion_slippage_bps = conversion_slippage_bps;
    config.active = true;
    config.total_positions = 0;
    config.bump = ctx.bumps.collateral_config;

    msg!("Collateral config initialized for mint: {}", config.mint);
    msg!("Max LTV: {} bps", max_ltv_bps);
    msg!("Liquidation threshold: {} bps", liquidation_threshold_bps);
    msg!("Base rate: {} bps | Slope1: {} bps | Slope2: {} bps | Optimal util: {} bps",
        base_rate_bps, rate_slope1_bps, rate_slope2_bps, optimal_utilization_bps);

    Ok(())
}

#[derive(Accounts)]
pub struct InitializeCollateralConfig<'info> {
    /// Protocol authority — only this account can add collateral types.
    #[account(mut)]
    pub authority: Signer<'info>,

    /// The collateral config PDA being created.
    #[account(
        init,
        payer = authority,
        space = CollateralConfig::SIZE,
        seeds = [b"collateral_config", collateral_mint.key().as_ref()],
        bump
    )]
    pub collateral_config: Account<'info, CollateralConfig>,

    /// The collateral token mint being whitelisted.
    pub collateral_mint: Account<'info, Mint>,

    /// CHECK: Pyth price feed for this collateral type.
    pub pyth_price_feed: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}
