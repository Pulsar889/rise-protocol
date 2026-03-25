use anchor_lang::prelude::*;
use crate::state::{GovernanceConfig, VeLock, GaugeVote, GaugeAllocation};
use crate::errors::GovernanceError;

pub fn handler(ctx: Context<VoteGauge>, gauges: Vec<GaugeAllocation>) -> Result<()> {
    let current_slot = Clock::get()?.slot;
    let epoch = Clock::get()?.epoch;
    let lock = &ctx.accounts.lock;

    require!(
        current_slot < lock.lock_end_slot,
        GovernanceError::LockExpired
    );

    let total_weight: u32 = gauges.iter().map(|g| g.weight_bps as u32).sum();
    require!(total_weight == 10_000, GovernanceError::InvalidGaugeWeights);
    require!(gauges.len() <= 8, GovernanceError::InvalidGaugeWeights);

    let gauge_vote = &mut ctx.accounts.gauge_vote;
    gauge_vote.owner = ctx.accounts.user.key();
    gauge_vote.epoch = epoch;
    gauge_vote.bump = ctx.bumps.gauge_vote;

    let mut gauge_array = [GaugeAllocation::default(); 8];
    for (i, g) in gauges.iter().enumerate() {
        gauge_array[i] = *g;
    }
    gauge_vote.gauges = gauge_array;

    msg!("Gauge votes recorded for epoch {}", epoch);
    msg!("veRISE voting weight: {}", lock.current_verise(current_slot));

    Ok(())
}

#[derive(Accounts)]
pub struct VoteGauge<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        seeds = [b"governance_config"],
        bump = config.bump
    )]
    pub config: Account<'info, GovernanceConfig>,

    #[account(
        seeds = [b"ve_lock", user.key().as_ref(), &[lock.nonce]],
        bump = lock.bump,
        constraint = lock.owner == user.key()
    )]
    pub lock: Account<'info, VeLock>,

    #[account(
        init_if_needed,
        payer = user,
        space = GaugeVote::SIZE,
        seeds = [b"gauge_vote", user.key().as_ref()],
        bump
    )]
    pub gauge_vote: Account<'info, GaugeVote>,

    pub system_program: Program<'info, System>,
}
