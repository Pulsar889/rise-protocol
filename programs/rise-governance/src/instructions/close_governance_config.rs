use anchor_lang::prelude::*;
use crate::state::GovernanceConfig;
use crate::errors::GovernanceError;

/// Authority-only: close the governance config and reclaim rent.
/// Used for devnet re-initialization when the RISE mint needs to change.
pub fn handler(_ctx: Context<CloseGovernanceConfig>) -> Result<()> {
    msg!("governance_config closed, rent reclaimed");
    Ok(())
}

#[derive(Accounts)]
pub struct CloseGovernanceConfig<'info> {
    #[account(
        mut,
        constraint = authority.key() == config.authority @ GovernanceError::Unauthorized
    )]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"governance_config"],
        bump = config.bump,
        close = authority
    )]
    pub config: Account<'info, GovernanceConfig>,
}
