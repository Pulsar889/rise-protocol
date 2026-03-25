use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Mint, MintTo, SetAuthority, Token, TokenAccount},
};
use anchor_spl::token::spl_token::instruction::AuthorityType;
use crate::state::{GovernanceConfig, VeLock};
use crate::errors::GovernanceError;
use crate::nft_cpi;

pub fn handler(
    ctx: Context<LockRise>,
    amount: u64,
    lock_slots: u64,
    nonce: u8,
) -> Result<()> {
    // Save the config AccountInfo before any mutable borrow so we can pass it
    // to the metadata CPI later without a borrow-checker conflict.
    let config_account_info = ctx.accounts.config.to_account_info();

    let config = &mut ctx.accounts.config;

    require!(amount > 0, GovernanceError::ZeroAmount);
    require!(lock_slots >= config.min_lock_slots, GovernanceError::LockTooShort);
    require!(lock_slots <= config.max_lock_slots, GovernanceError::LockTooLong);

    let current_slot = Clock::get()?.slot;

    // Calculate veRISE amount
    let verise_amount = GovernanceConfig::calculate_verise(amount, lock_slots)
        .ok_or(GovernanceError::MathOverflow)?;

    require!(verise_amount > 0, GovernanceError::ZeroAmount);

    // Transfer RISE from user to lock vault
    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            token::Transfer {
                from: ctx.accounts.user_rise_account.to_account_info(),
                to: ctx.accounts.rise_vault.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        ),
        amount,
    )?;

    // Read revenue index from treasury account data
    let revenue_index = {
        let treasury_data = ctx.accounts.treasury.try_borrow_data()?;
        if treasury_data.len() >= 124 {
            let mut bytes = [0u8; 16];
            bytes.copy_from_slice(&treasury_data[108..124]);
            u128::from_le_bytes(bytes)
        } else {
            0u128
        }
    };

    // ── Assign sequential lock number ────────────────────────────────────────
    config.lock_count = config.lock_count
        .checked_add(1)
        .ok_or(GovernanceError::MathOverflow)?;
    let lock_number = config.lock_count;

    // ── Mint veRISE Lock NFT ─────────────────────────────────────────────────
    //
    // nft_mint was initialized by Anchor as a fresh 0-decimal SPL mint with
    // the user as mint_authority (user is a direct signer).
    //
    //   1. Mint exactly 1 token to the user's ATA.
    //   2. Remove the mint authority → enforces a permanent max supply of 1.
    //   3. Create the Metaplex metadata account named "veRISE Lock #N".

    // 1. Mint 1 token → user ATA
    token::mint_to(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint: ctx.accounts.nft_mint.to_account_info(),
                to: ctx.accounts.user_nft_ata.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        ),
        1,
    )?;

    // 2. Create Metaplex metadata while user is still mint_authority.
    //    Must happen before set_authority removes it — Metaplex validates the
    //    mint_authority signer matches the current authority on the mint.
    let nft_name = format!("veRISE Lock #{}", lock_number);
    nft_cpi::create_metadata_v3(
        &ctx.accounts.nft_metadata.to_account_info(),
        &ctx.accounts.nft_mint.to_account_info(),
        &ctx.accounts.user.to_account_info(),          // mint_authority (signer)
        &ctx.accounts.user.to_account_info(),          // payer
        &config_account_info,                          // update_authority = config PDA
        &ctx.accounts.system_program.to_account_info(),
        &ctx.accounts.rent.to_account_info(),
        &ctx.accounts.token_metadata_program.to_account_info(),
        &nft_name,
        "veRISE",
        "",   // URI — updateable later via update_metadata
        None, // no PDA signer needed; user is mint_authority and is a direct signer
    )?;

    // 3. Remove mint authority (set to None) — enforces permanent max supply of 1.
    token::set_authority(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            SetAuthority {
                account_or_mint: ctx.accounts.nft_mint.to_account_info(),
                current_authority: ctx.accounts.user.to_account_info(),
            },
        ),
        AuthorityType::MintTokens,
        None,
    )?;

    // ── Initialize lock state ─────────────────────────────────────────────────
    let lock = &mut ctx.accounts.lock;
    lock.owner         = ctx.accounts.user.key();
    lock.rise_locked   = amount;
    lock.verise_amount = verise_amount;
    lock.lock_start_slot = current_slot;
    lock.lock_end_slot   = current_slot
        .checked_add(lock_slots)
        .ok_or(GovernanceError::MathOverflow)?;
    lock.last_revenue_index  = revenue_index;
    lock.total_revenue_claimed = 0;
    lock.nft_mint    = ctx.accounts.nft_mint.key();
    lock.lock_number = lock_number;
    lock.nonce = nonce;
    lock.bump  = ctx.bumps.lock;

    // Update total veRISE supply
    config.total_verise = config.total_verise
        .checked_add(verise_amount as u128)
        .ok_or(GovernanceError::MathOverflow)?;

    msg!("Locked {} RISE for {} slots", amount, lock_slots);
    msg!("veRISE issued: {}", verise_amount);
    msg!("Lock expires at slot: {}", lock.lock_end_slot);
    msg!("NFT minted: {} (veRISE Lock #{})", lock.nft_mint, lock_number);
    msg!("Total veRISE supply: {}", config.total_verise);

    Ok(())
}

#[derive(Accounts)]
#[instruction(amount: u64, lock_slots: u64, nonce: u8)]
pub struct LockRise<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [b"governance_config"],
        bump = config.bump
    )]
    pub config: Account<'info, GovernanceConfig>,

    #[account(
        init,
        payer = user,
        space = VeLock::SIZE,
        seeds = [b"ve_lock", user.key().as_ref(), &[nonce]],
        bump
    )]
    pub lock: Account<'info, VeLock>,

    // ── RISE token accounts ──────────────────────────────────────────────────

    #[account(
        mut,
        constraint = user_rise_account.mint == config.rise_mint,
        constraint = user_rise_account.owner == user.key()
    )]
    pub user_rise_account: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [b"rise_vault"],
        bump,
        constraint = rise_vault.mint == config.rise_mint
    )]
    pub rise_vault: Box<Account<'info, TokenAccount>>,

    // ── veRISE Lock NFT ──────────────────────────────────────────────────────

    /// Fresh NFT mint keypair generated by the client for each lock position.
    /// Decimals = 0; user is initial mint_authority (revoked after minting 1).
    #[account(
        init,
        payer = user,
        mint::decimals = 0,
        mint::authority = user,
        mint::freeze_authority = user,
    )]
    pub nft_mint: Box<Account<'info, Mint>>,

    /// User's Associated Token Account for this NFT mint.
    #[account(
        init,
        payer = user,
        associated_token::mint = nft_mint,
        associated_token::authority = user,
    )]
    pub user_nft_ata: Box<Account<'info, TokenAccount>>,

    /// Metaplex metadata PDA — validated by the Token Metadata program.
    /// CHECK: PDA owned and validated by the Token Metadata program.
    #[account(mut)]
    pub nft_metadata: UncheckedAccount<'info>,

    /// Metaplex Token Metadata program.
    /// CHECK: Verified against the known program ID.
    #[account(address = nft_cpi::TOKEN_METADATA_PROGRAM_ID)]
    pub token_metadata_program: UncheckedAccount<'info>,

    // ── Misc ─────────────────────────────────────────────────────────────────

    /// CHECK: Read-only reference to staking treasury for revenue index.
    pub treasury: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}
