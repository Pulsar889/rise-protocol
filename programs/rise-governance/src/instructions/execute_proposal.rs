use anchor_lang::prelude::*;
use crate::state::{GovernanceConfig, Proposal};
use crate::errors::GovernanceError;

pub fn handler(ctx: Context<ExecuteProposal>) -> Result<()> {
    let current_slot = Clock::get()?.slot;
    let config = &mut ctx.accounts.config;
    let proposal = &mut ctx.accounts.proposal;

    // Voting must have ended
    require!(
        current_slot > proposal.voting_end_slot,
        GovernanceError::VotingNotEnded
    );

    // Timelock must have elapsed
    require!(
        current_slot >= proposal.execution_slot,
        GovernanceError::TimelockNotElapsed
    );

    // Must not already be executed
    require!(!proposal.executed, GovernanceError::AlreadyExecuted);

    // Must have passed
    require!(
        proposal.is_passed(config.total_verise, config.quorum_bps),
        GovernanceError::ProposalFailed
    );

    // Mark as executed and free the active proposal slot
    proposal.executed = true;
    config.active_proposal_count = config.active_proposal_count.saturating_sub(1);

    msg!("Proposal #{} executed", proposal.index);
    msg!("Votes for: {}", proposal.votes_for);
    msg!("Votes against: {}", proposal.votes_against);
    msg!("Total veRISE: {}", config.total_verise);

    // Note: actual on-chain execution of the proposal's target instruction
    // is handled via a separate CPI in production. For v1 we mark as executed
    // and the off-chain system reads this to apply the change via governance multisig.

    Ok(())
}

#[derive(Accounts)]
pub struct ExecuteProposal<'info> {
    /// Anyone can execute a passed proposal.
    pub caller: Signer<'info>,

    #[account(
        mut,
        seeds = [b"governance_config"],
        bump = config.bump
    )]
    pub config: Account<'info, GovernanceConfig>,

    #[account(
        mut,
        constraint = !proposal.executed @ GovernanceError::AlreadyExecuted
    )]
    pub proposal: Account<'info, Proposal>,
}
