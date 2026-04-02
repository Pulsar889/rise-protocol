use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;
use crate::state::{StakeRewardsConfig, UserStakeRewards, GlobalPool};
use crate::errors::StakingError;

/// Creates a UserStakeRewards account for the caller, registering them for
/// RISE staking rewards going forward.
///
/// reward_debt is set to the caller's current riseSOL balance * reward_per_token
/// so they only accrue rewards from this point on — they cannot claim backdated
/// rewards for riseSOL they held before registering.
pub fn handler(ctx: Context<RegisterStakeRewards>) -> Result<()> {
    let reward_per_token = ctx.accounts.stake_rewards_config.reward_per_token;
    let current_amount = ctx.accounts.user_rise_sol_account.amount;

    let user_rewards = &mut ctx.accounts.user_stake_rewards;
    user_rewards.owner = ctx.accounts.user.key();
    user_rewards.pending_rewards = 0;
    user_rewards.total_claimed = 0;
    user_rewards.bump = ctx.bumps.user_stake_rewards;

    // Sync debt to current balance — prevents claiming backdated rewards.
    user_rewards.sync_debt(reward_per_token, current_amount)?;

    msg!("Registered stake rewards for {}", ctx.accounts.user.key());
    msg!("Initial riseSOL amount: {}", current_amount);

    Ok(())
}

#[derive(Accounts)]
pub struct RegisterStakeRewards<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        seeds = [b"global_pool"],
        bump = pool.bump
    )]
    pub pool: Account<'info, GlobalPool>,

    #[account(
        seeds = [b"stake_rewards_config"],
        bump = stake_rewards_config.bump
    )]
    pub stake_rewards_config: Account<'info, StakeRewardsConfig>,

    #[account(
        init,
        payer = user,
        space = UserStakeRewards::SIZE,
        seeds = [b"user_stake_rewards", user.key().as_ref()],
        bump
    )]
    pub user_stake_rewards: Account<'info, UserStakeRewards>,

    /// The user's riseSOL token account — used to read their current balance.
    #[account(
        constraint = user_rise_sol_account.mint == pool.rise_sol_mint @ StakingError::Unauthorized,
        constraint = user_rise_sol_account.owner == user.key() @ StakingError::Unauthorized,
    )]
    pub user_rise_sol_account: Account<'info, TokenAccount>,

    pub system_program: Program<'info, System>,
}
