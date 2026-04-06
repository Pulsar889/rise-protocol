use anchor_lang::prelude::*;
use state::GaugeAllocation;

declare_id!("CtMKhgY5xKiwLB5jmQ44PRF9QsUqXqSbiyVbFsidskHz");

pub mod state;
pub mod instructions;
pub mod errors;
pub mod nft_cpi;

use instructions::*;

#[program]
pub mod rise_governance {
    use super::*;

    pub fn initialize_governance(
        ctx: Context<InitializeGovernance>,
        proposal_threshold: u64,
        quorum_bps: u16,
    ) -> Result<()> {
        instructions::initialize_governance::handler(ctx, proposal_threshold, quorum_bps)
    }

    pub fn initialize_rise_vault(ctx: Context<InitializeRiseVault>) -> Result<()> {
        instructions::initialize_rise_vault::handler(ctx)
    }

    pub fn lock_rise(
        ctx: Context<LockRise>,
        amount: u64,
        lock_slots: u64,
        nonce: u8,
    ) -> Result<()> {
        instructions::lock_rise::handler(ctx, amount, lock_slots, nonce)
    }

    pub fn unlock_rise(ctx: Context<UnlockRise>) -> Result<()> {
        instructions::unlock_rise::handler(ctx)
    }

    pub fn extend_lock(ctx: Context<ExtendLock>, additional_slots: u64) -> Result<()> {
        instructions::extend_lock::handler(ctx, additional_slots)
    }

    pub fn vote_gauge(ctx: Context<VoteGauge>, gauges: Vec<GaugeAllocation>) -> Result<()> {
        instructions::vote_gauge::handler(ctx, gauges)
    }

    pub fn create_proposal(
        ctx: Context<CreateProposal>,
        description: [u8; 128],
        target_program: Pubkey,
    ) -> Result<()> {
        instructions::create_proposal::handler(ctx, description, target_program)
    }

    pub fn cast_vote(ctx: Context<CastVote>, vote_for: bool) -> Result<()> {
        instructions::cast_vote::handler(ctx, vote_for)
    }

    pub fn execute_proposal(ctx: Context<ExecuteProposal>) -> Result<()> {
        instructions::execute_proposal::handler(ctx)
    }

    pub fn claim_revenue_share(
        ctx: Context<ClaimRevenueShare>,
    ) -> Result<()> {
        instructions::claim_revenue_share::handler(ctx)
    }

    pub fn update_governance_config(
        ctx: Context<UpdateGovernanceConfig>,
        proposal_threshold: Option<u64>,
        quorum_bps: Option<u16>,
        voting_period_slots: Option<u64>,
        timelock_slots: Option<u64>,
    ) -> Result<()> {
        instructions::update_governance_config::handler(
            ctx, proposal_threshold, quorum_bps, voting_period_slots, timelock_slots,
        )
    }
}
