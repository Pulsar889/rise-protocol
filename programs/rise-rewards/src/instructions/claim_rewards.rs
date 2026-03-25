use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::state::{RewardsConfig, Gauge, UserStake};
use crate::errors::RewardsError;

pub fn handler(ctx: Context<ClaimRewards>) -> Result<()> {
    let gauge = &ctx.accounts.gauge;
    let stake = &mut ctx.accounts.user_stake;

    // Calculate newly accrued rewards
    let newly_accrued = (stake.lp_amount as u128)
        .checked_mul(gauge.reward_per_token)
        .ok_or(RewardsError::MathOverflow)?
        .checked_div(Gauge::REWARD_SCALE)
        .ok_or(RewardsError::MathOverflow)?
        .saturating_sub(stake.reward_debt) as u64;

    let total_claimable = stake.pending_rewards
        .checked_add(newly_accrued)
        .ok_or(RewardsError::MathOverflow)?;

    require!(total_claimable > 0, RewardsError::NoRewardsToClaim);

    // Transfer RISE rewards to user using config bump stored in account
    let config_bump = ctx.accounts.config.bump;
    let seeds = &[b"rewards_config".as_ref(), &[config_bump]];
    let signer = &[&seeds[..]];

    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        Transfer {
            from: ctx.accounts.rewards_vault.to_account_info(),
            to: ctx.accounts.user_rise_account.to_account_info(),
            authority: ctx.accounts.config.to_account_info(),
        },
        signer,
    );
    token::transfer(cpi_ctx, total_claimable)?;

    // Update stake
    stake.pending_rewards = 0;
    stake.reward_debt = (stake.lp_amount as u128)
        .checked_mul(gauge.reward_per_token)
        .ok_or(RewardsError::MathOverflow)?
        .checked_div(Gauge::REWARD_SCALE)
        .ok_or(RewardsError::MathOverflow)?;

    msg!("Claimed {} RISE rewards", total_claimable);

    Ok(())
}

#[derive(Accounts)]
pub struct ClaimRewards<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        seeds = [b"rewards_config"],
        bump = config.bump
    )]
    pub config: Account<'info, RewardsConfig>,

    #[account(
        seeds = [b"gauge", gauge.pool.as_ref()],
        bump = gauge.bump
    )]
    pub gauge: Account<'info, Gauge>,

    #[account(
        mut,
        seeds = [b"user_stake", user.key().as_ref(), gauge.key().as_ref()],
        bump = user_stake.bump,
        constraint = user_stake.owner == user.key()
    )]
    pub user_stake: Account<'info, UserStake>,

    #[account(
        mut,
        seeds = [b"rewards_vault"],
        bump,
        constraint = rewards_vault.mint == config.rise_mint
    )]
    pub rewards_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = user_rise_account.mint == config.rise_mint,
        constraint = user_rise_account.owner == user.key()
    )]
    pub user_rise_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}
