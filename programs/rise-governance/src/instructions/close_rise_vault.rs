use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount, Mint, Burn, CloseAccount};
use crate::state::GovernanceConfig;
use crate::errors::GovernanceError;

/// Authority-only: burn any remaining tokens in rise_vault, then close it and
/// reclaim rent.  Used for devnet re-initialization when the wrong RISE mint
/// was used during setup.
///
/// After calling this you can call `initialize_rise_vault` again with a fresh
/// `governance_config` that holds the correct mint.
pub fn handler(ctx: Context<CloseRiseVault>) -> Result<()> {
    let config = &ctx.accounts.config;
    let seeds: &[&[u8]] = &[b"governance_config", &[config.bump]];
    let signer = &[seeds];

    // Burn whatever tokens are in the vault (they belong to the wrong mint and
    // have no value — burning is the cleanest way to empty the account).
    let balance = ctx.accounts.rise_vault.amount;
    if balance > 0 {
        anchor_spl::token::burn(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.rise_vault_mint.to_account_info(),
                    from: ctx.accounts.rise_vault.to_account_info(),
                    authority: ctx.accounts.config.to_account_info(),
                },
                signer,
            ),
            balance,
        )?;
        msg!("Burned {} tokens from rise_vault (wrong mint)", balance);
    }

    // Close the token account and send rent lamports back to authority.
    anchor_spl::token::close_account(CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        CloseAccount {
            account: ctx.accounts.rise_vault.to_account_info(),
            destination: ctx.accounts.authority.to_account_info(),
            authority: ctx.accounts.config.to_account_info(),
        },
        signer,
    ))?;

    msg!("rise_vault closed, rent returned to authority");
    Ok(())
}

#[derive(Accounts)]
pub struct CloseRiseVault<'info> {
    #[account(
        mut,
        constraint = authority.key() == config.authority @ GovernanceError::Unauthorized
    )]
    pub authority: Signer<'info>,

    #[account(
        seeds = [b"governance_config"],
        bump = config.bump
    )]
    pub config: Account<'info, GovernanceConfig>,

    /// The vault to close.  Seeds verified so we can't be pointed at an
    /// arbitrary token account.
    #[account(
        mut,
        seeds = [b"rise_vault"],
        bump,
        token::mint = rise_vault_mint,
        token::authority = config,
    )]
    pub rise_vault: Account<'info, TokenAccount>,

    /// The mint currently held by the vault (needed for the burn CPI).
    #[account(mut)]
    pub rise_vault_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
}
