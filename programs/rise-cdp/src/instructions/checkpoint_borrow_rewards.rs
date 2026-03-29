use anchor_lang::prelude::*;
use crate::state::BorrowRewardsConfig;
use crate::errors::CdpError;

/// Permissionless crank — advances the global reward_per_token accumulator.
///
/// Computes how many RISE tokens have been emitted since the last checkpoint
/// and distributes them proportionally across total CDP debt:
///   reward_per_token += emissions * REWARD_SCALE / total_cdp_debt
///
/// Can be called by anyone at any time; calling more frequently increases
/// reward distribution granularity.
pub fn handler(ctx: Context<CheckpointBorrowRewards>) -> Result<()> {
    let config = &mut ctx.accounts.borrow_rewards_config;
    let current_slot = Clock::get()?.slot;

    let slots_elapsed = current_slot.saturating_sub(config.last_checkpoint_slot);

    if slots_elapsed == 0 {
        msg!("No slots elapsed since last checkpoint — nothing to do");
        return Ok(());
    }

    if config.total_cdp_debt == 0 {
        // No borrowers — advance the slot pointer but don't accumulate.
        config.last_checkpoint_slot = current_slot;
        msg!("No CDP debt — advancing checkpoint slot");
        return Ok(());
    }

    // emissions = epoch_emissions * slots_elapsed / slots_per_epoch
    let emissions = (config.epoch_emissions as u128)
        .checked_mul(slots_elapsed as u128)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(config.slots_per_epoch as u128)
        .ok_or(CdpError::MathOverflow)?;

    if emissions > 0 {
        // reward_per_token += emissions * REWARD_SCALE / total_cdp_debt
        let reward_increase = emissions
            .checked_mul(BorrowRewardsConfig::REWARD_SCALE)
            .ok_or(CdpError::MathOverflow)?
            .checked_div(config.total_cdp_debt as u128)
            .ok_or(CdpError::MathOverflow)?;

        config.reward_per_token = config.reward_per_token
            .checked_add(reward_increase)
            .ok_or(CdpError::MathOverflow)?;

        msg!("Checkpoint: {} slots elapsed, {} RISE emitted", slots_elapsed, emissions);
        msg!("New reward_per_token: {}", config.reward_per_token);
    }

    config.last_checkpoint_slot = current_slot;

    Ok(())
}

#[derive(Accounts)]
pub struct CheckpointBorrowRewards<'info> {
    /// Permissionless — anyone can call this crank.
    pub caller: Signer<'info>,

    #[account(
        mut,
        seeds = [b"borrow_rewards_config"],
        bump = borrow_rewards_config.bump
    )]
    pub borrow_rewards_config: Account<'info, BorrowRewardsConfig>,
}
