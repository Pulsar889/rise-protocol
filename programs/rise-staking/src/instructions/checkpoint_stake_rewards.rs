use anchor_lang::prelude::*;
use crate::state::StakeRewardsConfig;
use crate::errors::StakingError;

/// Permissionless crank — advances the global reward_per_token accumulator.
///
/// Computes how many RISE tokens have been emitted since the last checkpoint
/// and distributes them proportionally across the total riseSOL staking supply:
///   reward_per_token += emissions * REWARD_SCALE / total_staking_supply
///
/// Can be called by anyone at any time; calling more frequently gives stakers
/// more granular accrual (important when the supply changes often).
pub fn handler(ctx: Context<CheckpointStakeRewards>) -> Result<()> {
    let config = &mut ctx.accounts.stake_rewards_config;
    let current_slot = Clock::get()?.slot;

    let slots_elapsed = current_slot.saturating_sub(config.last_checkpoint_slot);

    if slots_elapsed == 0 {
        msg!("No slots elapsed since last checkpoint — nothing to do");
        return Ok(());
    }

    if config.total_staking_supply == 0 {
        // No stakers — advance the slot pointer but don't accumulate.
        config.last_checkpoint_slot = current_slot;
        msg!("No staking supply — advancing checkpoint slot");
        return Ok(());
    }

    // emissions = epoch_emissions * slots_elapsed / slots_per_epoch
    let emissions = (config.epoch_emissions as u128)
        .checked_mul(slots_elapsed as u128)
        .ok_or(StakingError::MathOverflow)?
        .checked_div(config.slots_per_epoch as u128)
        .ok_or(StakingError::MathOverflow)?;

    if emissions > 0 {
        // reward_per_token += emissions * REWARD_SCALE / total_staking_supply
        let reward_increase = emissions
            .checked_mul(StakeRewardsConfig::REWARD_SCALE)
            .ok_or(StakingError::MathOverflow)?
            .checked_div(config.total_staking_supply as u128)
            .ok_or(StakingError::MathOverflow)?;

        config.reward_per_token = config.reward_per_token
            .checked_add(reward_increase)
            .ok_or(StakingError::MathOverflow)?;

        msg!("Checkpoint: {} slots elapsed, {} RISE emitted", slots_elapsed, emissions);
        msg!("New reward_per_token: {}", config.reward_per_token);
    }

    config.last_checkpoint_slot = current_slot;

    Ok(())
}

#[derive(Accounts)]
pub struct CheckpointStakeRewards<'info> {
    /// Permissionless — anyone can call this crank.
    pub caller: Signer<'info>,

    #[account(
        mut,
        seeds = [b"stake_rewards_config"],
        bump = stake_rewards_config.bump
    )]
    pub stake_rewards_config: Account<'info, StakeRewardsConfig>,
}
