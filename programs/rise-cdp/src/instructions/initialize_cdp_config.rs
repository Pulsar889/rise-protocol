use anchor_lang::prelude::*;
use crate::state::CdpConfig;
use crate::errors::CdpError;

pub fn handler(ctx: Context<InitializeCdpConfig>, debt_ceiling_multiplier_bps: u32) -> Result<()> {
    require!(debt_ceiling_multiplier_bps > 0, CdpError::ZeroAmount);

    let config = &mut ctx.accounts.cdp_config;
    config.authority = ctx.accounts.authority.key();
    config.cdp_rise_sol_minted = 0;
    config.debt_ceiling_multiplier_bps = debt_ceiling_multiplier_bps;
    config.bump = ctx.bumps.cdp_config;

    msg!("CDP config initialized");
    msg!("Debt ceiling multiplier: {} bps ({}x staking supply)",
        debt_ceiling_multiplier_bps,
        debt_ceiling_multiplier_bps / 10_000
    );

    Ok(())
}

#[derive(Accounts)]
pub struct InitializeCdpConfig<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = CdpConfig::SIZE,
        seeds = [b"cdp_config"],
        bump
    )]
    pub cdp_config: Account<'info, CdpConfig>,

    pub system_program: Program<'info, System>,
}
