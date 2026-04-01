use anchor_lang::prelude::*;
use crate::state::{RewardsConfig, Gauge};
use crate::errors::RewardsError;

/// Authority-only: set a gauge's weight in basis points.
/// Call this after tallying gauge votes off-chain each epoch.
/// All gauge weights must sum to 10,000 bps — enforced off-chain by the authority.
pub fn handler(ctx: Context<SetGaugeWeight>, weight_bps: u16) -> Result<()> {
    require!(weight_bps <= 10_000, RewardsError::InvalidWeight);
    ctx.accounts.gauge.weight_bps = weight_bps;
    msg!("Gauge {} weight set to {} bps", ctx.accounts.gauge.key(), weight_bps);
    Ok(())
}

#[derive(Accounts)]
pub struct SetGaugeWeight<'info> {
    #[account(
        constraint = authority.key() == config.authority @ RewardsError::Unauthorized
    )]
    pub authority: Signer<'info>,

    #[account(
        seeds = [b"rewards_config"],
        bump = config.bump,
    )]
    pub config: Account<'info, RewardsConfig>,

    #[account(
        mut,
        seeds = [b"gauge", gauge.pool.as_ref()],
        bump = gauge.bump,
    )]
    pub gauge: Account<'info, Gauge>,
}
