use anchor_lang::prelude::*;
use crate::state::CdpConfig;
use crate::errors::CdpError;

/// Update the global CDP debt ceiling multiplier. Authority or governance only.
/// multiplier_bps: e.g. 30000 = 3x staking supply, 20000 = 2x, 40000 = 4x.
pub fn handler(ctx: Context<UpdateDebtCeiling>, multiplier_bps: u32) -> Result<()> {
    require!(multiplier_bps > 0, CdpError::ZeroAmount);

    let config = &mut ctx.accounts.cdp_config;
    let old = config.debt_ceiling_multiplier_bps;
    config.debt_ceiling_multiplier_bps = multiplier_bps;

    msg!("Debt ceiling updated: {} bps → {} bps", old, multiplier_bps);

    Ok(())
}

#[derive(Accounts)]
pub struct UpdateDebtCeiling<'info> {
    #[account(
        constraint = authority.key() == cdp_config.authority
    )]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Account<'info, CdpConfig>,
}
