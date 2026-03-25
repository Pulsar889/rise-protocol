use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::state::{Gauge, UserStake};
use crate::errors::RewardsError;

pub fn handler(ctx: Context<WithdrawLp>, amount: u64) -> Result<()> {
    require!(amount > 0, RewardsError::ZeroAmount);
    require!(ctx.accounts.user_stake.lp_amount >= amount, RewardsError::InsufficientBalance);

    let gauge = &mut ctx.accounts.gauge;
    let stake = &mut ctx.accounts.user_stake;

    // Settle pending rewards before withdrawal
    let pending = (stake.lp_amount as u128)
        .checked_mul(gauge.reward_per_token)
        .ok_or(RewardsError::MathOverflow)?
        .checked_div(Gauge::REWARD_SCALE)
        .ok_or(RewardsError::MathOverflow)?
        .saturating_sub(stake.reward_debt) as u64;

    stake.pending_rewards = stake.pending_rewards
        .checked_add(pending)
        .ok_or(RewardsError::MathOverflow)?;

    // Transfer LP tokens back to user
    let pool_key = gauge.pool.clone();
    let vault_bump = ctx.bumps.gauge_lp_vault;
    let seeds = &[b"gauge_lp_vault".as_ref(), pool_key.as_ref(), &[vault_bump]];
    let signer = &[&seeds[..]];

    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        Transfer {
            from: ctx.accounts.gauge_lp_vault.to_account_info(),
            to: ctx.accounts.user_lp_account.to_account_info(),
            authority: ctx.accounts.gauge_lp_vault.to_account_info(),
        },
        signer,
    );
    token::transfer(cpi_ctx, amount)?;

    // Update stake
    stake.lp_amount = stake.lp_amount
        .checked_sub(amount)
        .ok_or(RewardsError::MathOverflow)?;

    stake.reward_debt = (stake.lp_amount as u128)
        .checked_mul(gauge.reward_per_token)
        .ok_or(RewardsError::MathOverflow)?
        .checked_div(Gauge::REWARD_SCALE)
        .ok_or(RewardsError::MathOverflow)?;

    // Update gauge total
    gauge.total_lp_deposited = gauge.total_lp_deposited
        .checked_sub(amount)
        .ok_or(RewardsError::MathOverflow)?;

    msg!("Withdrew {} LP tokens from gauge", amount);
    msg!("Pending rewards: {}", stake.pending_rewards);

    Ok(())
}

#[derive(Accounts)]
pub struct WithdrawLp<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
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
        constraint = user_lp_account.owner == user.key()
    )]
    pub user_lp_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"gauge_lp_vault", gauge.pool.as_ref()],
        bump
    )]
    pub gauge_lp_vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}
