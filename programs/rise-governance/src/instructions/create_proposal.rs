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

    // Sum current_verise across ALL provided locks.
    // Each lock is passed via remaining_accounts and validated manually:
    //   1. Account must be owned by this program
    //   2. Deserialise as VeLock (skip 8-byte Anchor discriminator)
    //   3. lock.owner must match the proposer
    //   4. PDA re-derivation must match the account key (prevents spoofed accounts)
    require!(!ctx.remaining_accounts.is_empty(), GovernanceError::InsufficientVeRise);

    let mut total_verise: u64 = 0;
    for acc in ctx.remaining_accounts.iter() {
        require!(acc.owner == ctx.program_id, GovernanceError::InvalidConfig);
        let data = acc.try_borrow_data()?;
        require!(data.len() >= VeLock::SIZE, GovernanceError::InvalidConfig);
        let lock: VeLock = AnchorDeserialize::deserialize(&mut &data[8..])?;
        require!(lock.owner == ctx.accounts.proposer.key(), GovernanceError::Unauthorized);
        // Verify this is a genuine VeLock PDA — not a spoofed account
        let expected = Pubkey::create_program_address(
            &[b"ve_lock", ctx.accounts.proposer.key().as_ref(), &[lock.nonce], &[lock.bump]],
            ctx.program_id,
        ).map_err(|_| error!(GovernanceError::InvalidConfig))?;
        require!(expected == *acc.key, GovernanceError::InvalidConfig);
        total_verise = total_verise.saturating_add(lock.current_verise(current_slot));
    }

    require!(total_verise >= config.proposal_threshold, GovernanceError::InsufficientVeRise);

    // Enforce active proposal cap
    require!(
        config.active_proposal_count < GovernanceConfig::MAX_ACTIVE_PROPOSALS,
        GovernanceError::TooManyActiveProposals
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
    // Proposal PDA seeds use config.proposal_count at creation time, which must equal
    // proposal.index. This ordering must be preserved: proposal.index is assigned before
    // config.proposal_count is incremented, so that execute_proposal, close_proposal, and
    // cast_vote — which derive the PDA using proposal.index — resolve to the same address.
    proposal.index = config.proposal_count;
    proposal.bump = ctx.bumps.proposal;

    // Increment proposal count and active proposal count
    config.proposal_count = config.proposal_count
        .checked_add(1)
        .ok_or(GovernanceError::MathOverflow)?;
    config.active_proposal_count = config.active_proposal_count
        .checked_add(1)
        .ok_or(GovernanceError::MathOverflow)?;

    msg!("Proposal #{} created", proposal.index);
    msg!("Voting ends at slot: {}", proposal.voting_end_slot);
    msg!("Executable at slot: {}", proposal.execution_slot);
    msg!("Total proposer veRISE: {}", total_verise);

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
        init,
        payer = proposer,
        space = Proposal::SIZE,
        seeds = [b"proposal", &config.proposal_count.to_le_bytes()],
        bump
    )]
    pub proposal: Account<'info, Proposal>,

    pub system_program: Program<'info, System>,
}
