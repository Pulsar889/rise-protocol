use anchor_lang::prelude::*;
use crate::state::{GovernanceConfig, VeLock};
use crate::errors::GovernanceError;

pub fn handler(ctx: Context<ExtendLock>, additional_slots: u64) -> Result<()> {
    let current_slot = Clock::get()?.slot;
    let config = &mut ctx.accounts.config;
    let lock = &mut ctx.accounts.lock;

    require!(additional_slots > 0, GovernanceError::ZeroAmount);
    require!(
        current_slot < lock.lock_end_slot,
        GovernanceError::LockExpired
    );

    let new_end_slot = lock.lock_end_slot
        .checked_add(additional_slots)
        .ok_or(GovernanceError::MathOverflow)?;

    let total_slots = new_end_slot
        .checked_sub(current_slot)
        .ok_or(GovernanceError::MathOverflow)?;

    require!(total_slots <= config.max_lock_slots, GovernanceError::LockTooLong);

    // Recalculate veRISE based on new remaining duration
    let new_verise = GovernanceConfig::calculate_verise(lock.rise_locked, total_slots)
        .ok_or(GovernanceError::MathOverflow)?;

    let old_verise = lock.verise_amount;

    // Update total veRISE supply
    config.total_verise = config.total_verise
        .saturating_sub(old_verise as u128)
        .checked_add(new_verise as u128)
        .ok_or(GovernanceError::MathOverflow)?;

    // Update lock
    lock.lock_end_slot = new_end_slot;
    lock.lock_start_slot = current_slot;
    lock.verise_amount = new_verise;

    msg!("Lock extended by {} slots", additional_slots);
    msg!("New end slot: {}", new_end_slot);
    msg!("Old veRISE: {} -> New veRISE: {}", old_verise, new_verise);
    msg!("Total veRISE supply: {}", config.total_verise);

    Ok(())
}

#[derive(Accounts)]
pub struct ExtendLock<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [b"governance_config"],
        bump = config.bump
    )]
    pub config: Account<'info, GovernanceConfig>,

    #[account(
        mut,
        seeds = [b"ve_lock", user.key().as_ref(), &[lock.nonce]],
        bump = lock.bump,
        constraint = lock.owner == user.key()
    )]
    pub lock: Account<'info, VeLock>,
}
