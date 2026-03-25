use anchor_lang::prelude::*;
use crate::state::RewardsConfig;
use crate::errors::RewardsError;

pub fn handler(ctx: Context<SetEpochEmissions>, epoch_emissions: u64) -> Result<()> {
    require!(epoch_emissions > 0, RewardsError::ZeroAmount);

    ctx.accounts.config.epoch_emissions = epoch_emissions;

    msg!("Epoch emissions updated to: {} RISE", epoch_emissions);

    Ok(())
}

#[derive(Accounts)]
pub struct SetEpochEmissions<'info> {
    #[account(
        constraint = authority.key() == config.authority @ RewardsError::Unauthorized
    )]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"rewards_config"],
        bump = config.bump
    )]
    pub config: Account<'info, RewardsConfig>,
}
