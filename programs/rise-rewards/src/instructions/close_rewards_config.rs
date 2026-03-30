use anchor_lang::prelude::*;
use crate::state::RewardsConfig;
use crate::errors::RewardsError;

/// Closes the rewards config, reclaiming rent.
/// Authority only — call this before re-initializing with a new RISE mint.
pub fn handler(_ctx: Context<CloseRewardsConfig>) -> Result<()> {
    msg!("Rewards config closed successfully");
    Ok(())
}

#[derive(Accounts)]
pub struct CloseRewardsConfig<'info> {
    #[account(
        mut,
        constraint = authority.key() == config.authority @ RewardsError::Unauthorized
    )]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"rewards_config"],
        bump = config.bump,
        close = authority,
    )]
    pub config: Account<'info, RewardsConfig>,
}
