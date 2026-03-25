use anchor_lang::prelude::*;
use anchor_spl::token::Mint;
use crate::state::RewardsConfig;
use crate::errors::RewardsError;

pub fn handler(
    ctx: Context<InitializeRewards>,
    epoch_emissions: u64,
) -> Result<()> {
    require!(epoch_emissions > 0, RewardsError::ZeroAmount);

    let config = &mut ctx.accounts.config;
    let current_slot = Clock::get()?.slot;

    config.authority = ctx.accounts.authority.key();
    config.rise_mint = ctx.accounts.rise_mint.key();
    config.epoch_emissions = epoch_emissions;
    config.current_epoch = 0;
    config.epoch_start_slot = current_slot;
    config.slots_per_epoch = RewardsConfig::SLOTS_PER_EPOCH;
    config.gauge_count = 0;
    config.bump = ctx.bumps.config;

    msg!("Rewards program initialized");
    msg!("RISE mint: {}", config.rise_mint);
    msg!("Epoch emissions: {} RISE", epoch_emissions);
    msg!("Slots per epoch: {}", config.slots_per_epoch);

    Ok(())
}

#[derive(Accounts)]
pub struct InitializeRewards<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = RewardsConfig::SIZE,
        seeds = [b"rewards_config"],
        bump
    )]
    pub config: Account<'info, RewardsConfig>,

    pub rise_mint: Account<'info, Mint>,

    pub system_program: Program<'info, System>,
}
