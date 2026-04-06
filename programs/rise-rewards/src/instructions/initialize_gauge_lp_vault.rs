use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount, Mint};
use crate::state::{RewardsConfig, Gauge};
use crate::errors::RewardsError;

/// Creates the LP token vault for a gauge.
/// Must be called once after `create_gauge` before any user can call `deposit_lp`.
/// Authority only.
pub fn handler(_ctx: Context<InitializeGaugeLpVault>) -> Result<()> {
    msg!("Gauge LP vault initialized: {}", _ctx.accounts.gauge_lp_vault.key());
    Ok(())
}

#[derive(Accounts)]
pub struct InitializeGaugeLpVault<'info> {
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
        seeds = [b"gauge", gauge.pool.as_ref()],
        bump = gauge.bump,
    )]
    pub gauge: Account<'info, Gauge>,

    /// The LP token mint for this pool. The vault will only accept this token.
    pub lp_mint: Account<'info, Mint>,

    /// The vault that holds staked LP tokens for this gauge.
    /// Authority is the vault PDA itself — matches the signing pattern in withdraw_lp.
    #[account(
        init,
        payer = authority,
        token::mint = lp_mint,
        token::authority = gauge_lp_vault,
        seeds = [b"gauge_lp_vault", gauge.pool.as_ref()],
        bump,
    )]
    pub gauge_lp_vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
