use anchor_lang::prelude::*;
use crate::state::GlobalPool;
use crate::errors::StakingError;

/// Called by the CDP program after transferring fee revenue into pool_vault.
/// Immediately credits the deposited SOL to total_sol_staked and updates the
/// exchange rate so stakers benefit right away rather than waiting for the
/// next update_exchange_rate epoch crank.
///
/// # Security model (I-3)
///
/// This instruction is **fully permissionless** — any account can call it with
/// any `amount`, as long as the pool_vault already holds that many extra lamports.
/// The vault balance check (`vault_balance >= accounted + amount`) is the only
/// authorization gate. This is intentional: anyone who actually deposits SOL into
/// the pool vault should be able to have that deposit recognised immediately. A
/// malicious caller who does NOT first deposit SOL would fail the balance check,
/// so the worst a bad actor can do is call it harmlessly after a legitimate deposit
/// that the CDP program itself would have called anyway. There is no griefing risk.
pub fn handler(ctx: Context<CreditStakingRevenue>, amount: u64) -> Result<()> {
    require!(amount > 0, StakingError::ZeroAmount);

    let vault_balance = ctx.accounts.pool_vault.lamports() as u128;
    let pool = &ctx.accounts.global_pool;

    let accounted = pool
        .liquid_buffer_lamports
        .checked_add(pool.pending_withdrawals_lamports)
        .ok_or(StakingError::MathOverflow)?;

    require!(
        vault_balance >= accounted + amount as u128,
        StakingError::InsufficientLiquidity
    );

    let pool = &mut ctx.accounts.global_pool;

    pool.total_sol_staked = pool
        .total_sol_staked
        .checked_add(amount as u128)
        .ok_or(StakingError::MathOverflow)?;

    pool.liquid_buffer_lamports = pool
        .liquid_buffer_lamports
        .checked_add(amount as u128)
        .ok_or(StakingError::MathOverflow)?;

    if pool.staking_rise_sol_supply > 0 {
        pool.exchange_rate = pool
            .total_sol_staked
            .checked_mul(GlobalPool::RATE_SCALE)
            .ok_or(StakingError::MathOverflow)?
            .checked_div(pool.staking_rise_sol_supply)
            .ok_or(StakingError::MathOverflow)?;
    }

    msg!("Staking revenue credited: {} lamports", amount);
    msg!("New total_sol_staked:      {}", pool.total_sol_staked);
    msg!("New exchange_rate:         {}", pool.exchange_rate);

    Ok(())
}

#[derive(Accounts)]
pub struct CreditStakingRevenue<'info> {
    pub caller: Signer<'info>,

    #[account(
        mut,
        seeds = [b"global_pool"],
        bump = global_pool.bump
    )]
    pub global_pool: Account<'info, GlobalPool>,

    /// CHECK: Pool SOL vault — balance verified inside the handler.
    #[account(
        seeds = [b"pool_vault"],
        bump
    )]
    pub pool_vault: UncheckedAccount<'info>,
}
