use anchor_lang::prelude::*;
use crate::state::{RewardsConfig, Gauge};
use crate::errors::RewardsError;

pub fn handler(ctx: Context<CheckpointGauge>) -> Result<()> {
    let config = &mut ctx.accounts.config;
    let gauge = &mut ctx.accounts.gauge;
    let current_slot = Clock::get()?.slot;

    // Check if epoch has ended
    let slots_elapsed = current_slot.saturating_sub(config.epoch_start_slot);
    if slots_elapsed >= config.slots_per_epoch {
        // Advance epoch
        config.current_epoch = config.current_epoch
            .checked_add(1)
            .ok_or(RewardsError::MathOverflow)?;
        config.epoch_start_slot = current_slot;
        msg!("Epoch advanced to: {}", config.current_epoch);
    }

    // Skip if already checkpointed this epoch
    if gauge.last_checkpoint_epoch >= config.current_epoch {
        msg!("Gauge already checkpointed for epoch {}", config.current_epoch);
        return Ok(());
    }

    // Calculate RISE allocated to this gauge based on weight
    if gauge.weight_bps == 0 || gauge.total_lp_deposited == 0 {
        gauge.last_checkpoint_epoch = config.current_epoch;
        msg!("Gauge has no weight or deposits — skipping distribution");
        return Ok(());
    }

    let gauge_emissions = (config.epoch_emissions as u128)
        .checked_mul(gauge.weight_bps as u128)
        .ok_or(RewardsError::MathOverflow)?
        .checked_div(10_000)
        .ok_or(RewardsError::MathOverflow)? as u64;

    // Update reward_per_token
    // reward_per_token += gauge_emissions * REWARD_SCALE / total_lp_deposited
    let reward_increase = (gauge_emissions as u128)
        .checked_mul(Gauge::REWARD_SCALE)
        .ok_or(RewardsError::MathOverflow)?
        .checked_div(gauge.total_lp_deposited as u128)
        .ok_or(RewardsError::MathOverflow)?;

    gauge.reward_per_token = gauge.reward_per_token
        .checked_add(reward_increase)
        .ok_or(RewardsError::MathOverflow)?;

    gauge.total_distributed = gauge.total_distributed
        .checked_add(gauge_emissions)
        .ok_or(RewardsError::MathOverflow)?;

    gauge.last_checkpoint_epoch = config.current_epoch;

    msg!("Gauge checkpointed for epoch {}", config.current_epoch);
    msg!("RISE allocated: {}", gauge_emissions);
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
