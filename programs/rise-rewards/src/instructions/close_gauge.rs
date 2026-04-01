use anchor_lang::prelude::*;
use crate::state::Gauge;
use crate::errors::RewardsError;

/// Authority-only: closes an individual Gauge account and reclaims rent.
/// Use this to clean up orphaned gauges after a config reset.
pub fn handler(_ctx: Context<CloseGauge>) -> Result<()> {
    msg!("Gauge closed: {}", _ctx.accounts.gauge.key());
    Ok(())
}

#[derive(Accounts)]
pub struct CloseGauge<'info> {
    #[account(
        mut,
        constraint = authority.key() == config.authority @ RewardsError::Unauthorized
    )]
    pub authority: Signer<'info>,

    /// CHECK: only used for authority check — may be a freshly reset config
    /// or any account at the rewards_config PDA. We verify authority above.
    #[account(
        seeds = [b"rewards_config"],
        bump,
    )]
    pub config: Account<'info, crate::state::RewardsConfig>,

    #[account(
        mut,
        close = authority,
    )]
    pub gauge: Account<'info, Gauge>,
}
