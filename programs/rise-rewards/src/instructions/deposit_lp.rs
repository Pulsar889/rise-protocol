use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::state::{Gauge, UserStake};
use crate::errors::RewardsError;

pub fn handler(ctx: Context<DepositLp>, amount: u64) -> Result<()> {
    require!(amount >= 1_000, RewardsError::ZeroAmount);
    require!(ctx.accounts.gauge.active, RewardsError::GaugeNotActive);

    let gauge = &mut ctx.accounts.gauge;
    let stake = &mut ctx.accounts.user_stake;

    // Settle pending rewards before updating stake
    if stake.lp_amount > 0 {
        let pending = u64::try_from(
            (stake.lp_amount as u128)
                .checked_mul(gauge.reward_per_token)
                .ok_or(RewardsError::MathOverflow)?
                .checked_div(Gauge::REWARD_SCALE)
                .ok_or(RewardsError::MathOverflow)?
                .checked_sub(stake.reward_debt)
                .ok_or(RewardsError::MathOverflow)?
        ).map_err(|_| RewardsError::MathOverflow)?;

        stake.pending_rewards = stake.pending_rewards
            .checked_add(pending)
            .ok_or(RewardsError::MathOverflow)?;
    }

    // Transfer LP tokens from user to gauge vault
    let cpi_ctx = CpiContext::new(
        ctx.accounts.token_program.to_account_info(),
        Transfer {
            from: ctx.accounts.user_lp_account.to_account_info(),
            to: ctx.accounts.gauge_lp_vault.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        },
    );
    token::transfer(cpi_ctx, amount)?;

    // Update stake
    stake.owner = ctx.accounts.user.key();
    stake.gauge = gauge.key();
    stake.lp_amount = stake.lp_amount
        .checked_add(amount)
        .ok_or(RewardsError::MathOverflow)?;

    // Update reward debt
    stake.reward_debt = (stake.lp_amount as u128)
        .checked_mul(gauge.reward_per_token)
        .ok_or(RewardsError::MathOverflow)?
        .checked_div(Gauge::REWARD_SCALE)
        .ok_or(RewardsError::MathOverflow)?;

    stake.bump = ctx.bumps.user_stake;

    // Update gauge total
    gauge.total_lp_deposited = gauge.total_lp_deposited
        .checked_add(amount)
        .ok_or(RewardsError::MathOverflow)?;

    msg!("Deposited {} LP tokens into gauge", amount);
    msg!("Total LP in gauge: {}", gauge.total_lp_deposited);

    Ok(())
}

#[derive(Accounts)]
pub struct DepositLp<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [b"gauge", gauge.pool.as_ref()],
        bump = gauge.bump
    )]
    pub gauge: Account<'info, Gauge>,

    #[account(
        init_if_needed,
        payer = user,
        space = UserStake::SIZE,
        seeds = [b"user_stake", user.key().as_ref(), gauge.key().as_ref()],
        bump
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
    pub system_program: Program<'info, System>,
}
