use anchor_lang::prelude::*;
use crate::state::{GovernanceConfig, Proposal};
use crate::errors::GovernanceError;

/// Authority-only: close an executed or expired proposal and reclaim rent.
pub fn handler(ctx: Context<CloseProposal>) -> Result<()> {
    let current_slot = Clock::get()?.slot;
    let proposal = &ctx.accounts.proposal;

    require!(
        proposal.executed || current_slot > proposal.voting_end_slot,
        GovernanceError::VotingNotEnded
    );

    msg!("Proposal #{} closed, rent reclaimed", proposal.index);
    Ok(())
}

#[derive(Accounts)]
pub struct CloseProposal<'info> {
    #[account(
        mut,
        constraint = authority.key() == config.authority @ GovernanceError::InvalidConfig
    )]
    pub authority: Signer<'info>,

    #[account(
        seeds = [b"governance_config"],
        bump = config.bump,
    )]
    pub config: Account<'info, GovernanceConfig>,

    #[account(
        mut,
        close = authority
    )]
    pub proposal: Account<'info, Proposal>,
}
