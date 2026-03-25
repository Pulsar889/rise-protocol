use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount, Mint};
use crate::state::GovernanceConfig;

pub fn handler(ctx: Context<InitializeRiseVault>) -> Result<()> {
    msg!("RISE vault initialized at: {}", ctx.accounts.rise_vault.key());
    Ok(())
}

#[derive(Accounts)]
pub struct InitializeRiseVault<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [b"governance_config"],
        bump = config.bump
    )]
    pub config: Account<'info, GovernanceConfig>,

    #[account(
        init,
        payer = authority,
        token::mint = rise_mint,
        token::authority = config,
        seeds = [b"rise_vault"],
        bump
    )]
    pub rise_vault: Account<'info, TokenAccount>,

    pub rise_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
