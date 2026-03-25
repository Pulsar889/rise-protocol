use anchor_lang::prelude::*;
use crate::state::{RewardsConfig, Gauge};
use crate::errors::RewardsError;

pub fn handler(ctx: Context<CreateGauge>, pool: Pubkey) -> Result<()> {
    let config = &mut ctx.accounts.config;
    let gauge = &mut ctx.accounts.gauge;

    gauge.pool = pool;
    gauge.index = config.gauge_count;
    gauge.weight_bps = 0;
    gauge.active = true;
    gauge.reward_per_token = 0;
    gauge.total_lp_deposited = 0;
    gauge.last_checkpoint_epoch = config.current_epoch;
    gauge.total_distributed = 0;
    gauge.bump = ctx.bumps.gauge;

    config.gauge_count = config.gauge_count
        .checked_add(1)
        .ok_or(RewardsError::MathOverflow)?;

    msg!("Gauge #{} created for pool: {}", gauge.index, pool);

    Ok(())
}

#[derive(Accounts)]
#[instruction(pool: Pubkey)]
pub struct CreateGauge<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"rewards_config"],
        bump = config.bump,
        constraint = config.authority == authority.key() @ RewardsError::Unauthorized
    )]
    pub config: Account<'info, RewardsConfig>,

    #[account(
        init,
        payer = authority,
        space = Gauge::SIZE,
        seeds = [b"gauge", pool.as_ref()],
        bump
    )]
    pub gauge: Account<'info, Gauge>,

    pub system_program: Program<'info, System>,
}
