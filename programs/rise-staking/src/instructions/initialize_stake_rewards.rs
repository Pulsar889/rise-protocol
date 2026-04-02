use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount, Mint};
use crate::state::{StakeRewardsConfig, GlobalPool};
use crate::errors::StakingError;

pub fn handler(
    ctx: Context<InitializeStakeRewards>,
    epoch_emissions: u64,
    slots_per_epoch: u64,
) -> Result<()> {
    require!(epoch_emissions > 0, StakingError::ZeroAmount);
    require!(slots_per_epoch > 0, StakingError::ZeroAmount);

    let config = &mut ctx.accounts.stake_rewards_config;
    let current_slot = Clock::get()?.slot;

    config.authority = ctx.accounts.authority.key();
    config.rise_mint = ctx.accounts.rise_mint.key();
    config.rewards_vault = ctx.accounts.rewards_vault.key();
    config.reward_per_token = 0;
    config.epoch_emissions = epoch_emissions;
    config.slots_per_epoch = slots_per_epoch;
    // Seed total_staking_supply from the pool's current staking supply so that
    // any stakers who existed before initialization are tracked correctly.
    config.total_staking_supply = ctx.accounts.pool.staking_rise_sol_supply as u64;
    config.last_checkpoint_slot = current_slot;
    config.bump = ctx.bumps.stake_rewards_config;

    msg!("Stake rewards initialized");
    msg!("RISE mint:       {}", config.rise_mint);
    msg!("Epoch emissions: {}", epoch_emissions);
    msg!("Slots per epoch: {}", slots_per_epoch);
    msg!("Initial staking supply: {}", config.total_staking_supply);

    Ok(())
}

#[derive(Accounts)]
pub struct InitializeStakeRewards<'info> {
    #[account(
        mut,
        constraint = authority.key() == pool.authority @ StakingError::Unauthorized
    )]
    pub authority: Signer<'info>,

    /// Global staking pool — used to verify authority and seed total_staking_supply.
    #[account(
        seeds = [b"global_pool"],
        bump = pool.bump
    )]
    pub pool: Account<'info, GlobalPool>,

    #[account(
        init,
        payer = authority,
        space = StakeRewardsConfig::SIZE,
        seeds = [b"stake_rewards_config"],
        bump
    )]
    pub stake_rewards_config: Account<'info, StakeRewardsConfig>,

    /// Token account PDA that will hold RISE for staker rewards.
    /// Authority is the stake_rewards_config PDA itself.
    #[account(
        init,
        payer = authority,
        token::mint = rise_mint,
        token::authority = stake_rewards_config,
        seeds = [b"stake_rewards_vault"],
        bump
    )]
    pub rewards_vault: Account<'info, TokenAccount>,

    pub rise_mint: Account<'info, Mint>,

    #[account(address = Token::id())]
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
