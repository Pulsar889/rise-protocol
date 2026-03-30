use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Mint, Burn, CloseAccount};
use crate::state::{BorrowRewardsConfig, CdpConfig};
use crate::errors::CdpError;

/// Closes the borrow-rewards config and vault, reclaiming rent.
/// Burns any remaining tokens in the vault before closing it.
/// Authority only — call this before re-initializing with a new RISE mint.
pub fn handler(ctx: Context<CloseBorrowRewards>) -> Result<()> {
    let config_bump = ctx.accounts.borrow_rewards_config.bump;
    let seeds: &[&[u8]] = &[b"borrow_rewards_config", &[config_bump]];
    let signer = &[seeds];

    // Burn any remaining balance so close_account won't fail.
    let remaining = ctx.accounts.rewards_vault.amount;
    if remaining > 0 {
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Burn {
                mint:      ctx.accounts.old_rise_mint.to_account_info(),
                from:      ctx.accounts.rewards_vault.to_account_info(),
                authority: ctx.accounts.borrow_rewards_config.to_account_info(),
            },
            signer,
        );
        token::burn(cpi_ctx, remaining)?;
        msg!("Burned {} stale RISE tokens from borrow_rewards_vault", remaining);
    }

    // Close the token vault — rent goes to authority.
    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        CloseAccount {
            account:     ctx.accounts.rewards_vault.to_account_info(),
            destination: ctx.accounts.authority.to_account_info(),
            authority:   ctx.accounts.borrow_rewards_config.to_account_info(),
        },
        signer,
    );
    token::close_account(cpi_ctx)?;

    // borrow_rewards_config is closed by the `close = authority` constraint.
    msg!("Borrow rewards config and vault closed successfully");
    Ok(())
}

#[derive(Accounts)]
pub struct CloseBorrowRewards<'info> {
    #[account(
        mut,
        constraint = authority.key() == cdp_config.authority @ CdpError::BorrowRewardsNotInitialized
    )]
    pub authority: Signer<'info>,

    #[account(seeds = [b"cdp_config"], bump = cdp_config.bump)]
    pub cdp_config: Account<'info, CdpConfig>,

    #[account(
        mut,
        seeds = [b"borrow_rewards_config"],
        bump = borrow_rewards_config.bump,
        close = authority,
    )]
    pub borrow_rewards_config: Account<'info, BorrowRewardsConfig>,

    #[account(
        mut,
        seeds = [b"borrow_rewards_vault"],
        bump,
        constraint = rewards_vault.mint == borrow_rewards_config.rise_mint,
    )]
    pub rewards_vault: Account<'info, TokenAccount>,

    /// The old RISE mint stored in the vault — needed for the burn CPI.
    #[account(
        mut,
        constraint = old_rise_mint.key() == borrow_rewards_config.rise_mint,
    )]
    pub old_rise_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
}
