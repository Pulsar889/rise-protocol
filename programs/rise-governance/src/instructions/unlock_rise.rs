use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Token, TokenAccount};
use crate::state::{GovernanceConfig, VeLock};
use crate::errors::GovernanceError;
use crate::nft_cpi;

pub fn handler(ctx: Context<UnlockRise>) -> Result<()> {
    let current_slot = Clock::get()?.slot;
    let lock = &ctx.accounts.lock;

    require!(
        current_slot >= lock.lock_end_slot,
        GovernanceError::LockNotExpired
    );

    let rise_to_return   = lock.rise_locked;
    let verise_to_remove = lock.verise_amount;

    // ── Burn the veRISE Lock NFT ─────────────────────────────────────────────
    //
    // The mint_authority was permanently removed when the lock was created, so
    // only the token holder can burn.  The token account is not frozen (no
    // master edition was created), so a plain spl-token burn works.
    nft_cpi::burn_nft_token(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.user_nft_ata.to_account_info(),
        &ctx.accounts.nft_mint.to_account_info(),
        &ctx.accounts.user.to_account_info(),
    )?;

    // Close the now-empty ATA and reclaim rent lamports.
    token::close_account(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account: ctx.accounts.user_nft_ata.to_account_info(),
                destination: ctx.accounts.user.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        ),
    )?;

    // ── Return RISE from vault to user ───────────────────────────────────────
    let vault_bump = ctx.bumps.rise_vault;
    let seeds = &[b"rise_vault".as_ref(), &[vault_bump]];
    let signer = &[&seeds[..]];

    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            token::Transfer {
                from: ctx.accounts.rise_vault.to_account_info(),
                to: ctx.accounts.user_rise_account.to_account_info(),
                authority: ctx.accounts.rise_vault.to_account_info(),
            },
            signer,
        ),
        rise_to_return,
    )?;

    // ── Update total veRISE supply ───────────────────────────────────────────
    let config = &mut ctx.accounts.config;
    config.total_verise = config.total_verise
        .saturating_sub(verise_to_remove as u128);

    msg!("Unlocked {} RISE", rise_to_return);
    msg!("Removed {} veRISE from supply", verise_to_remove);
    msg!("NFT burned: {}", ctx.accounts.nft_mint.key());
    msg!("Total veRISE supply: {}", config.total_verise);

    Ok(())
}

#[derive(Accounts)]
pub struct UnlockRise<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [b"governance_config"],
        bump = config.bump
    )]
    pub config: Account<'info, GovernanceConfig>,

    #[account(
        mut,
        seeds = [b"ve_lock", user.key().as_ref(), &[lock.nonce]],
        bump = lock.bump,
        constraint = lock.owner == user.key(),
        close = user
    )]
    pub lock: Account<'info, VeLock>,

    // ── RISE token accounts ──────────────────────────────────────────────────

    #[account(
        mut,
        constraint = user_rise_account.mint == config.rise_mint,
        constraint = user_rise_account.owner == user.key()
    )]
    pub user_rise_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"rise_vault"],
        bump,
        constraint = rise_vault.mint == config.rise_mint
    )]
    pub rise_vault: Account<'info, TokenAccount>,

    // ── veRISE Lock NFT ──────────────────────────────────────────────────────

    /// The NFT mint stored on the lock PDA — verified by address constraint.
    /// CHECK: Address verified against lock.nft_mint below.
    #[account(
        mut,
        address = lock.nft_mint
    )]
    pub nft_mint: UncheckedAccount<'info>,

    /// User's ATA holding the 1 NFT token.
    #[account(
        mut,
        constraint = user_nft_ata.mint  == lock.nft_mint     @ GovernanceError::ZeroAmount,
        constraint = user_nft_ata.owner == user.key()        @ GovernanceError::ZeroAmount,
        constraint = user_nft_ata.amount == 1                @ GovernanceError::ZeroAmount,
    )]
    pub user_nft_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}
