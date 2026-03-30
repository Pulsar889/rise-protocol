use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount, Mint};
use crate::state::RewardsConfig;
use crate::errors::RewardsError;

/// Creates the RISE token vault that backs LP gauge reward payouts.
/// Seeds: ["rewards_vault"] under the rise-rewards program.
/// Authority of the vault is the rewards_config PDA so it can sign transfers.
/// Call this once after initialize_rewards.
pub fn handler(_ctx: Context<InitializeRewardsVault>) -> Result<()> {
    msg!("Rewards vault initialized at: {}", _ctx.accounts.rewards_vault.key());
    Ok(())
}

#[derive(Accounts)]
pub struct InitializeRewardsVault<'info> {
    #[account(
        mut,
        constraint = authority.key() == config.authority @ RewardsError::Unauthorized
    )]
    pub authority: Signer<'info>,

    #[account(
        seeds = [b"rewards_config"],
        bump = config.bump,
    )]
    pub config: Account<'info, RewardsConfig>,

    #[account(
        init,
        payer = authority,
        token::mint = rise_mint,
        token::authority = config,
        seeds = [b"rewards_vault"],
        bump,
    )]
    pub rewards_vault: Account<'info, TokenAccount>,

    #[account(constraint = rise_mint.key() == config.rise_mint @ RewardsError::Unauthorized)]
    pub rise_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
