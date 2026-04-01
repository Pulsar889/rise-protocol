use anchor_lang::prelude::*;
use crate::state::GlobalPool;
use crate::errors::StakingError;

/// stake_lamports_total: the sum of all validator stake account balances, passed in
/// by the crank. Stake rewards compound in place — we read the balance rather than
/// withdrawing each epoch. Currently 0 until validator delegation is built; at that
/// point full stake reward accounting will be added alongside total_sol_in_stake tracking.
pub fn handler(ctx: Context<UpdateExchangeRate>, stake_lamports_total: u64) -> Result<()> {
    let pool = &mut ctx.accounts.pool;
    let current_epoch = Clock::get()?.epoch;

    if current_epoch <= pool.last_rate_update_epoch {
        msg!("Exchange rate already updated this epoch");
        return Ok(());
    }

    let vault_balance = ctx.accounts.pool_vault.lamports() as u128;

    // Liquid rewards = vault excess above the tracked liquid buffer and pending withdrawals.
    // Pending withdrawals sit in the vault but are committed to users — not staker capital.
    // Vault invariant: vault = liquid_buffer + pending_withdrawals + fee_excess
    let available = vault_balance
        .checked_sub(pool.pending_withdrawals_lamports)
        .ok_or(StakingError::MathOverflow)?;

    if available <= pool.liquid_buffer_lamports {
        msg!("No new rewards this epoch");
        pool.last_rate_update_epoch = current_epoch;
        return Ok(());
    }

    let rewards = available
        .checked_sub(pool.liquid_buffer_lamports)
        .ok_or(StakingError::MathOverflow)?;

    let protocol_fee = rewards
        .checked_mul(pool.protocol_fee_bps as u128)
        .ok_or(StakingError::MathOverflow)?
        .checked_div(10_000)
        .ok_or(StakingError::MathOverflow)?;

    let credited_rewards = rewards
        .checked_sub(protocol_fee)
        .ok_or(StakingError::MathOverflow)?;

    pool.total_sol_staked = pool
        .total_sol_staked
        .checked_add(credited_rewards)
        .ok_or(StakingError::MathOverflow)?;

    // Only credit the net rewards to liquid_buffer — protocol_fee stays as excess
    // above (liquid_buffer + pending_withdrawals), which collect_fees will sweep.
    pool.liquid_buffer_lamports = pool
        .liquid_buffer_lamports
        .checked_add(credited_rewards)
        .ok_or(StakingError::MathOverflow)?;

    if pool.staking_rise_sol_supply > 0 {
        // Snapshot current rate before overwriting so frontend can compute APY.
        pool.prev_exchange_rate = pool.exchange_rate;
        pool.prev_rate_update_slot = Clock::get()?.slot;

        pool.exchange_rate = pool
            .total_sol_staked
            .checked_mul(GlobalPool::RATE_SCALE)
            .ok_or(StakingError::MathOverflow)?
            .checked_div(pool.staking_rise_sol_supply)
            .ok_or(StakingError::MathOverflow)?;
    }

    pool.last_rate_update_epoch = current_epoch;

    msg!("Rewards: {} | Fee: {} | Credited: {} | New rate: {}",
        rewards, protocol_fee, credited_rewards, pool.exchange_rate);

    Ok(())
}

#[derive(Accounts)]
pub struct UpdateExchangeRate<'info> {
    pub caller: Signer<'info>,

    #[account(
        mut,
        seeds = [b"global_pool"],
        bump = pool.bump
    )]
    pub pool: Account<'info, GlobalPool>,

    /// CHECK: Pool SOL vault.
    #[account(
        seeds = [b"pool_vault"],
        bump
    )]
    pub pool_vault: UncheckedAccount<'info>,
}
