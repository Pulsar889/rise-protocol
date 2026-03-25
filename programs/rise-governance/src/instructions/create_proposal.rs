use anchor_lang::prelude::*;
use crate::state::{GovernanceConfig, VeLock, Proposal};
use crate::errors::GovernanceError;

pub fn handler(
    ctx: Context<CreateProposal>,
    description: [u8; 128],
    target_program: Pubkey,
) -> Result<()> {
    let current_slot = Clock::get()?.slot;
    let config = &mut ctx.accounts.config;
    let lock = &ctx.accounts.lock;

    // Check proposer has enough veRISE
    let current_verise = lock.current_verise(current_slot);
    require!(
        current_verise >= config.proposal_threshold,
        GovernanceError::InsufficientVeRise
    );

    // Initialize proposal
    let proposal = &mut ctx.accounts.proposal;
    proposal.proposer = ctx.accounts.proposer.key();
    proposal.description = description;
    proposal.target_program = target_program;
    proposal.voting_end_slot = current_slot
        .checked_add(config.voting_period_slots)
        .ok_or(GovernanceError::MathOverflow)?;
    proposal.execution_slot = proposal.voting_end_slot
        .checked_add(config.timelock_slots)
        .ok_or(GovernanceError::MathOverflow)?;
    proposal.votes_for = 0;
    proposal.votes_against = 0;
    proposal.executed = false;
    proposal.index = config.proposal_count;
    proposal.bump = ctx.bumps.proposal;

    // Increment proposal count
    config.proposal_count = config.proposal_count
        .checked_add(1)
        .ok_or(GovernanceError::MathOverflow)?;

    msg!("Proposal #{} created", proposal.index);
    msg!("Voting ends at slot: {}", proposal.voting_end_slot);
    msg!("Executable at slot: {}", proposal.execution_slot);
    msg!("Proposer veRISE: {}", current_verise);

    Ok(())
}

#[derive(Accounts)]
pub struct CreateProposal<'info> {
    #[account(mut)]
    pub proposer: Signer<'info>,

    #[account(
        mut,
        seeds = [b"governance_config"],
        bump = config.bump
    )]
    pub config: Account<'info, GovernanceConfig>,

    #[account(
        seeds = [b"ve_lock", proposer.key().as_ref(), &[lock.nonce]],
        bump = lock.bump,
        constraint = lock.owner == proposer.key()
    )]
    pub lock: Account<'info, VeLock>,

    #[account(
        init,
        payer = proposer,
        space = Proposal::SIZE,
        seeds = [b"proposal", &config.proposal_count.to_le_bytes()],
        bump
    )]
    pub proposal: Account<'info, Proposal>,

    pub system_program: Program<'info, System>,
}
