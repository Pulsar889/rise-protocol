use anchor_lang::prelude::*;
use crate::state::{GovernanceConfig, VeLock, Proposal, VoteRecord};
use crate::errors::GovernanceError;

pub fn handler(ctx: Context<CastVote>, vote_for: bool) -> Result<()> {
    let current_slot = Clock::get()?.slot;
    let lock = &ctx.accounts.lock;
    let proposal = &mut ctx.accounts.proposal;

    // Voting period must still be open
    require!(
        current_slot <= proposal.voting_end_slot,
        GovernanceError::VotingEnded
    );

    // Lock must not be expired
    require!(
        current_slot < lock.lock_end_slot,
        GovernanceError::LockExpired
    );

    // Get current veRISE weight
    let verise_weight = lock.current_verise(current_slot);
    require!(verise_weight > 0, GovernanceError::ZeroAmount);

    // Record vote
    let vote_record = &mut ctx.accounts.vote_record;
    vote_record.voter = ctx.accounts.voter.key();
    vote_record.lock = ctx.accounts.lock.key();
    vote_record.proposal = proposal.key();
    vote_record.verise_at_vote = verise_weight;
    vote_record.vote_for = vote_for;
    vote_record.bump = ctx.bumps.vote_record;

    // Tally vote
    if vote_for {
        proposal.votes_for = proposal.votes_for
            .checked_add(verise_weight as u128)
            .ok_or(GovernanceError::MathOverflow)?;
    } else {
        proposal.votes_against = proposal.votes_against
            .checked_add(verise_weight as u128)
            .ok_or(GovernanceError::MathOverflow)?;
    }

    msg!("Vote cast: {}", if vote_for { "FOR" } else { "AGAINST" });
    msg!("veRISE weight: {}", verise_weight);
    msg!("Proposal votes for: {}", proposal.votes_for);
    msg!("Proposal votes against: {}", proposal.votes_against);

    Ok(())
}

#[derive(Accounts)]
pub struct CastVote<'info> {
    #[account(mut)]
    pub voter: Signer<'info>,

    #[account(
        seeds = [b"governance_config"],
        bump = config.bump
    )]
    pub config: Account<'info, GovernanceConfig>,

    #[account(
        seeds = [b"ve_lock", voter.key().as_ref(), &[lock.nonce]],
        bump = lock.bump,
        constraint = lock.owner == voter.key()
    )]
    pub lock: Account<'info, VeLock>,

    #[account(
        mut,
        seeds = [b"proposal", &proposal.index.to_le_bytes()],
        bump = proposal.bump,
    )]
    pub proposal: Account<'info, Proposal>,

    #[account(
        init,
        payer = voter,
        space = VoteRecord::SIZE,
        seeds = [b"vote_record", lock.key().as_ref(), proposal.key().as_ref()],
        bump
    )]
    pub vote_record: Account<'info, VoteRecord>,

    pub system_program: Program<'info, System>,
}
