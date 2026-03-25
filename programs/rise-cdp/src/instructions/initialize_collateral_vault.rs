use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount, Mint};
use crate::state::CollateralConfig;
use crate::errors::CdpError;

pub fn handler(ctx: Context<InitializeCollateralVault>) -> Result<()> {
    msg!(
        "Collateral vault initialized for mint: {}",
        ctx.accounts.collateral_mint.key()
    );
    msg!("Vault address: {}", ctx.accounts.collateral_vault.key());
    Ok(())
}

#[derive(Accounts)]
pub struct InitializeCollateralVault<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        seeds = [b"collateral_config", collateral_mint.key().as_ref()],
        bump = collateral_config.bump,
        constraint = collateral_config.active @ CdpError::CollateralNotAccepted
    )]
    pub collateral_config: Account<'info, CollateralConfig>,

    pub collateral_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = authority,
        token::mint = collateral_mint,
        token::authority = collateral_vault,
        seeds = [b"collateral_vault", collateral_mint.key().as_ref()],
        bump
    )]
    pub collateral_vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
