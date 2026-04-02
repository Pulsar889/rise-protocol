use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::state::{StakeRewardsConfig, UserStakeRewards, GlobalPool};
use crate::errors::StakingError;

/// Claim accumulated RISE staking rewards.
///
/// Settles any newly-accrued rewards since the last checkpoint, then transfers
/// the full pending balance from the rewards vault to the staker's RISE ATA.
pub fn handler(ctx: Context<ClaimStakeRewards>) -> Result<()> {
    let reward_per_token = ctx.accounts.stake_rewards_config.reward_per_token;
    let current_amount = ctx.accounts.user_rise_sol_account.amount;

    let rewards = &mut ctx.accounts.user_stake_rewards;

    // Settle any rewards accrued since the last update.
    rewards.settle(reward_per_token)?;

    let total_claimable = rewards.pending_rewards;
    require!(total_claimable > 0, StakingError::NoRewardsToClaim);

    // Transfer RISE from the rewards vault.  The config PDA is the vault authority.
    let config_bump = ctx.accounts.stake_rewards_config.bump;
    let seeds = &[b"stake_rewards_config".as_ref(), &[config_bump]];
    let signer = &[&seeds[..]];

    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        Transfer {
            from:      ctx.accounts.rewards_vault.to_account_info(),
            to:        ctx.accounts.user_rise_account.to_account_info(),
            authority: ctx.accounts.stake_rewards_config.to_account_info(),
        },
        signer,
    );
    token::transfer(cpi_ctx, total_claimable)?;

    // Update accounting.
    rewards.pending_rewards = 0;
    rewards.total_claimed = rewards.total_claimed
        .checked_add(total_claimable)
        .ok_or(StakingError::MathOverflow)?;

    // Re-sync reward_debt to the current riseSOL balance.
    rewards.sync_debt(reward_per_token, current_amount)?;

    msg!("Claimed {} RISE staking rewards", total_claimable);
    msg!("Lifetime claimed: {}", rewards.total_claimed);

    Ok(())
}

#[derive(Accounts)]
pub struct ClaimStakeRewards<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        seeds = [b"global_pool"],
        bump = pool.bump
    )]
    pub pool: Box<Account<'info, GlobalPool>>,

    #[account(
        seeds = [b"stake_rewards_config"],
        bump = stake_rewards_config.bump
    )]
    pub stake_rewards_config: Box<Account<'info, StakeRewardsConfig>>,

    #[account(
        mut,
        seeds = [b"user_stake_rewards", user.key().as_ref()],
        bump = user_stake_rewards.bump,
        constraint = user_stake_rewards.owner == user.key() @ StakingError::Unauthorized,
    )]
    pub user_stake_rewards: Box<Account<'info, UserStakeRewards>>,

    /// riseSOL ATA — read to sync reward_debt after claim.
    #[account(
        constraint = user_rise_sol_account.mint == pool.rise_sol_mint @ StakingError::Unauthorized,
        constraint = user_rise_sol_account.owner == user.key() @ StakingError::Unauthorized,
    )]
    pub user_rise_sol_account: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [b"stake_rewards_vault"],
        bump,
        constraint = rewards_vault.mint == stake_rewards_config.rise_mint @ StakingError::Unauthorized,
    )]
    pub rewards_vault: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = user_rise_account.mint == stake_rewards_config.rise_mint @ StakingError::Unauthorized,
        constraint = user_rise_account.owner == user.key() @ StakingError::Unauthorized,
    )]
    pub user_rise_account: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
}
