use anchor_lang::prelude::*;
use anchor_spl::token::Mint;
use crate::state::GovernanceConfig;
use crate::errors::GovernanceError;

pub fn handler(
    ctx: Context<InitializeGovernance>,
    proposal_threshold: u64,
    quorum_bps: u16,
) -> Result<()> {
    require!(quorum_bps <= 10_000, GovernanceError::InvalidGaugeWeights);

    let config = &mut ctx.accounts.config;

    config.authority = ctx.accounts.authority.key();
    config.rise_mint = ctx.accounts.rise_mint.key();
    config.total_verise = 0;
    config.min_lock_slots = GovernanceConfig::SLOTS_PER_WEEK;
    config.max_lock_slots = GovernanceConfig::MAX_LOCK_SLOTS;
    config.proposal_threshold = proposal_threshold;
    config.voting_period_slots = GovernanceConfig::SLOTS_PER_WEEK; // 1 week voting
    config.timelock_slots = GovernanceConfig::SLOTS_PER_WEEK / 3;  // ~48 hours
    config.quorum_bps = quorum_bps;
    config.proposal_count = 0;
    config.lock_count = 0;
    config.bump = ctx.bumps.config;

    msg!("Governance initialized");
    msg!("RISE mint: {}", config.rise_mint);
    msg!("Proposal threshold: {} veRISE", proposal_threshold);
    msg!("Quorum: {} bps", quorum_bps);
    msg!("Min lock: {} slots", config.min_lock_slots);
    msg!("Max lock: {} slots", config.max_lock_slots);

    Ok(())
}

#[derive(Accounts)]
pub struct InitializeGovernance<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = GovernanceConfig::SIZE,
        seeds = [b"governance_config"],
        bump
    )]
    pub config: Account<'info, GovernanceConfig>,

    pub rise_mint: Account<'info, Mint>,

    pub system_program: Program<'info, System>,
}
