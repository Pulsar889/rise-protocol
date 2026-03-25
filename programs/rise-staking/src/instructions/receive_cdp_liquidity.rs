use anchor_lang::prelude::*;
use crate::state::GlobalPool;
use crate::errors::StakingError;

/// Called by the CDP program (or anyone) after converting seized collateral to SOL
/// and depositing it into pool_vault. Updates liquid_buffer_lamports to reflect
/// the new SOL so that queued withdrawal tickets can be paid out.
///
/// Permissionless — the only precondition is that the SOL is already in pool_vault.
/// The vault balance check prevents fake accounting without real deposits.
pub fn handler(ctx: Context<ReceiveCdpLiquidity>, sol_amount: u64) -> Result<()> {
    require!(sol_amount > 0, StakingError::ZeroAmount);

    // Verify the SOL was actually deposited into pool_vault before this call.
    // accounted_balance = liquid_buffer + pending_withdrawals (all SOL already spoken for).
    // The new deposit must push vault balance above that by at least sol_amount.
    let vault_balance = ctx.accounts.pool_vault.lamports();
    let pool = &ctx.accounts.global_pool;

    let accounted = pool.liquid_buffer_lamports
        .checked_add(pool.pending_withdrawals_lamports)
        .ok_or(StakingError::MathOverflow)?;

    require!(
        vault_balance as u128 >= accounted + sol_amount as u128,
        StakingError::InsufficientLiquidity
    );

    // Register the incoming SOL as liquid buffer.
    let pool = &mut ctx.accounts.global_pool;
    pool.liquid_buffer_lamports = pool.liquid_buffer_lamports
        .checked_add(sol_amount as u128)
        .ok_or(StakingError::MathOverflow)?;

    msg!("CDP liquidity registered: {} lamports", sol_amount);
    msg!("New liquid buffer: {} lamports", pool.liquid_buffer_lamports);

    Ok(())
}

#[derive(Accounts)]
pub struct ReceiveCdpLiquidity<'info> {
    /// Permissionless — anyone can call this once the SOL is in pool_vault.
    pub caller: Signer<'info>,

    #[account(
        mut,
        seeds = [b"global_pool"],
        bump = global_pool.bump
    )]
    pub global_pool: Account<'info, GlobalPool>,

    /// CHECK: Pool SOL vault — balance is read to verify deposit occurred.
    #[account(
        seeds = [b"pool_vault"],
        bump
    )]
    pub pool_vault: UncheckedAccount<'info>,
}
