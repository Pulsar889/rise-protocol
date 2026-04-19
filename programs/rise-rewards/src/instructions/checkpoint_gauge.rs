use anchor_lang::prelude::*;
use crate::state::{RewardsConfig, Gauge};
use crate::errors::RewardsError;

pub fn handler(ctx: Context<CheckpointGauge>) -> Result<()> {
    let config = &mut ctx.accounts.config;
    let gauge = &mut ctx.accounts.gauge;
    let current_slot = Clock::get()?.slot;

    // Advance epoch if enough slots have elapsed
    let slots_elapsed = current_slot.saturating_sub(config.epoch_start_slot);
    if slots_elapsed >= config.slots_per_epoch {
        config.current_epoch = config.current_epoch
            .checked_add(1)
            .ok_or(RewardsError::MathOverflow)?;
        config.epoch_start_slot = config.epoch_start_slot
            .saturating_add(config.slots_per_epoch);
        msg!("Epoch advanced to: {}", config.current_epoch);
    }

    // Skip if already checkpointed this epoch
    if gauge.last_checkpoint_epoch >= config.current_epoch {
        msg!("Gauge already checkpointed for epoch {}", config.current_epoch);
        return Ok(());
    }

    // This epoch's share of emissions based on gauge weight
    let epoch_allocation = if gauge.weight_bps == 0 {
        0u64
    } else {
        (config.epoch_emissions as u128)
            .checked_mul(gauge.weight_bps as u128)
            .ok_or(RewardsError::MathOverflow)?
            .checked_div(10_000)
            .ok_or(RewardsError::MathOverflow)? as u64
    };

    // Total to distribute = this epoch's allocation + any rolled-over emissions
    let total_to_distribute = epoch_allocation
        .checked_add(gauge.pending_emissions)
        .ok_or(RewardsError::MathOverflow)?;

    if gauge.total_lp_deposited == 0 {
        // No depositors — roll the full amount forward to the next epoch
        gauge.pending_emissions = total_to_distribute;
        gauge.last_checkpoint_epoch = config.current_epoch;
        msg!("No deposits — rolling over {} RISE to next epoch", total_to_distribute);
        return Ok(());
    }

    // Depositors exist — distribute everything (current epoch + rollover)
    if total_to_distribute > 0 {
        let reward_increase = (total_to_distribute as u128)
            .checked_mul(Gauge::REWARD_SCALE)
            .ok_or(RewardsError::MathOverflow)?
            .checked_div(gauge.total_lp_deposited as u128)
            .ok_or(RewardsError::MathOverflow)?;

        gauge.reward_per_token = gauge.reward_per_token
            .checked_add(reward_increase)
            .ok_or(RewardsError::MathOverflow)?;

        gauge.total_distributed = gauge.total_distributed
            .checked_add(total_to_distribute)
            .ok_or(RewardsError::MathOverflow)?;

        gauge.pending_emissions = 0;

        msg!("Distributed {} RISE ({} epoch + {} rollover)",
            total_to_distribute, epoch_allocation, total_to_distribute - epoch_allocation);
    }

    gauge.last_checkpoint_epoch = config.current_epoch;
    msg!("New reward_per_token: {}", gauge.reward_per_token);

    Ok(())
}

#[derive(Accounts)]
pub struct CheckpointGauge<'info> {
    pub caller: Signer<'info>,

    #[account(
        mut,
        seeds = [b"rewards_config"],
        bump = config.bump
    )]
    pub config: Account<'info, RewardsConfig>,

    #[account(
        mut,
        seeds = [b"gauge", gauge.pool.as_ref()],
        bump = gauge.bump
    )]
    pub gauge: Account<'info, Gauge>,
}
