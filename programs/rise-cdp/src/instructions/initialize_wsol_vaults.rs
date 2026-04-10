use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount, Mint};
use crate::state::CdpConfig;
use crate::errors::CdpError;

/// One-time deployment step: creates the two protocol WSOL vaults used by repay_debt
/// and repay_debt_rise_sol. Call this once after initialize_cdp_config.
pub fn handler(ctx: Context<InitializeWsolVaults>) -> Result<()> {
    msg!("cdp_wsol_vault initialized:        {}", ctx.accounts.cdp_wsol_vault.key());
    msg!("cdp_wsol_buyback_vault initialized: {}", ctx.accounts.cdp_wsol_buyback_vault.key());
    Ok(())
}

#[derive(Accounts)]
pub struct InitializeWsolVaults<'info> {
    #[account(
        mut,
        constraint = authority.key() == cdp_config.authority @ CdpError::Unauthorized
    )]
    pub authority: Signer<'info>,

    #[account(
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Account<'info, CdpConfig>,

    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,

    /// Protocol WSOL buffer used by repay_debt (SPL → SOL swap output).
    #[account(
        init,
        payer = authority,
        token::mint = wsol_mint,
        token::authority = cdp_config,
        seeds = [b"cdp_wsol_vault"],
        bump
    )]
    pub cdp_wsol_vault: Account<'info, TokenAccount>,

    /// Protocol WSOL buyback vault used by repay_debt and repay_debt_rise_sol (shortfall path).
    #[account(
        init,
        payer = authority,
        token::mint = wsol_mint,
        token::authority = cdp_config,
        seeds = [b"cdp_wsol_buyback_vault"],
        bump
    )]
    pub cdp_wsol_buyback_vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
