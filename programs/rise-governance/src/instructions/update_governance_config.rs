use anchor_lang::prelude::*;
use crate::state::GovernanceConfig;
use crate::errors::GovernanceError;

/// Update governance config parameters. Authority only.
/// Pass None for any field to leave it unchanged.
pub fn handler(
    ctx: Context<UpdateGovernanceConfig>,
    proposal_threshold: Option<u64>,
    quorum_bps: Option<u16>,
    voting_period_slots: Option<u64>,
    timelock_slots: Option<u64>,
) -> Result<()> {
    let config = &mut ctx.accounts.config;

    if let Some(threshold) = proposal_threshold {
        config.proposal_threshold = threshold;
        msg!("Proposal threshold updated to: {}", threshold);
    }

    if let Some(quorum) = quorum_bps {
        require!(quorum <= 10_000, GovernanceError::InvalidGaugeWeights);
        config.quorum_bps = quorum;
        msg!("Quorum updated to: {} bps", quorum);
    }

    if let Some(voting) = voting_period_slots {
        config.voting_period_slots = voting;
        msg!("Voting period updated to: {} slots", voting);
    }

    if let Some(timelock) = timelock_slots {
        config.timelock_slots = timelock;
        msg!("Timelock updated to: {} slots", timelock);
    }

    Ok(())
}

#[derive(Accounts)]
pub struct UpdateGovernanceConfig<'info> {
    #[account(constraint = authority.key() == config.authority)]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"governance_config"],
        bump = config.bump
    )]
    pub config: Account<'info, GovernanceConfig>,
}
