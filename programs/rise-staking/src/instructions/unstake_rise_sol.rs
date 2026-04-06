use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, Mint, Token, TokenAccount};
use crate::state::{GlobalPool, WithdrawalTicket, StakeRewardsConfig, UserStakeRewards};
use crate::errors::StakingError;

/// Burns riseSOL and creates a WithdrawalTicket redeemable after UNSTAKE_EPOCH_DELAY epochs.
/// The SOL amount is locked in at the current exchange rate.
pub fn handler(ctx: Context<UnstakeRiseSol>, rise_sol_amount: u64, nonce: u8) -> Result<()> {
    require!(!ctx.accounts.pool.paused, StakingError::PoolPaused);
    require!(rise_sol_amount > 0, StakingError::ZeroAmount);

    // ── Stake rewards: settle pending before balance changes ──────────────────
    if let Some(user_rewards) = ctx.accounts.user_stake_rewards.as_mut() {
        let reward_per_token = ctx.accounts.stake_rewards_config
            .as_ref()
            .map(|c| c.reward_per_token)
            .unwrap_or(0);

        let old_amount = ctx.accounts.user_rise_sol_account.amount;
        let new_amount = old_amount.saturating_sub(rise_sol_amount);
        user_rewards.settle(reward_per_token)?;
        user_rewards.sync_debt(reward_per_token, new_amount)?;
    }

    // ── Update stake_rewards_config supply ────────────────────────────────────
    if let Some(stake_rewards_config) = ctx.accounts.stake_rewards_config.as_mut() {
        stake_rewards_config.total_staking_supply = stake_rewards_config
            .total_staking_supply
            .saturating_sub(rise_sol_amount);
    }

    let pool = &mut ctx.accounts.pool;

    let sol_amount = pool
        .rise_sol_to_sol(rise_sol_amount)
        .ok_or(StakingError::MathOverflow)?;

    require!(sol_amount > 0, StakingError::ZeroAmount);

    // Burn riseSOL immediately
    let cpi_ctx = CpiContext::new(
        ctx.accounts.token_program.to_account_info(),
        Burn {
            mint: ctx.accounts.rise_sol_mint.to_account_info(),
            from: ctx.accounts.user_rise_sol_account.to_account_info(),
            authority: ctx.accounts.user.to_account_info(),
        },
    );
    token::burn(cpi_ctx, rise_sol_amount)?;

    // Update pool accounting — SOL moves from liquid buffer to pending withdrawals
    pool.staking_rise_sol_supply = pool
        .staking_rise_sol_supply
        .checked_sub(rise_sol_amount as u128)
        .ok_or(StakingError::MathOverflow)?;

    pool.total_sol_staked = pool
        .total_sol_staked
        .checked_sub(sol_amount as u128)
        .ok_or(StakingError::MathOverflow)?;

    pool.liquid_buffer_lamports = pool
        .liquid_buffer_lamports
        .checked_sub(sol_amount as u128)
        .ok_or(StakingError::MathOverflow)?;

    pool.pending_withdrawals_lamports = pool
        .pending_withdrawals_lamports
        .checked_add(sol_amount as u128)
        .ok_or(StakingError::MathOverflow)?;

    // Write ticket
    let current_epoch = Clock::get()?.epoch;
    let ticket = &mut ctx.accounts.ticket;
    ticket.owner = ctx.accounts.user.key();
    ticket.sol_amount = sol_amount;
    ticket.claimable_epoch = current_epoch + GlobalPool::UNSTAKE_EPOCH_DELAY;
    ticket.nonce = nonce;
    ticket.bump = ctx.bumps.ticket;

    msg!(
        "Unstake queued: burned {} riseSOL, {} lamports claimable at epoch {}",
        rise_sol_amount,
        sol_amount,
        ticket.claimable_epoch,
    );

    Ok(())
}

#[derive(Accounts)]
#[instruction(rise_sol_amount: u64, nonce: u8)]
pub struct UnstakeRiseSol<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [b"global_pool"],
        bump = pool.bump
    )]
    pub pool: Account<'info, GlobalPool>,

    #[account(
        init,
        payer = user,
        space = WithdrawalTicket::SIZE,
        seeds = [b"withdrawal_ticket", user.key().as_ref(), &[nonce]],
        bump
    )]
    pub ticket: Account<'info, WithdrawalTicket>,

    #[account(
        mut,
        address = pool.rise_sol_mint
    )]
    pub rise_sol_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = user_rise_sol_account.mint == pool.rise_sol_mint,
        constraint = user_rise_sol_account.owner == user.key()
    )]
    pub user_rise_sol_account: Account<'info, TokenAccount>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,

    // ── Optional stake rewards accounts ──────────────────────────────────────

    #[account(
        mut,
        seeds = [b"stake_rewards_config"],
        bump
    )]
    pub stake_rewards_config: Option<Account<'info, StakeRewardsConfig>>,

    #[account(
        mut,
        seeds = [b"user_stake_rewards", user.key().as_ref()],
        bump
    )]
    pub user_stake_rewards: Option<Account<'info, UserStakeRewards>>,
}
