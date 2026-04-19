use anchor_lang::prelude::*;
use crate::state::{CdpConfig, BorrowRewards};
use crate::errors::CdpError;

/// Closes an old-format CdpPosition account whose on-chain size no longer matches
/// the current struct (e.g. created before a struct field was added). Because the
/// account can't be deserialized as the current struct, it is passed as AccountInfo
/// and validated manually. The corresponding BorrowRewards account is also closed.
///
/// Authority-gated — only the CDP config authority can call this (devnet cleanup).
pub fn handler(ctx: Context<CloseStalePosition>) -> Result<()> {
    let pos = &ctx.accounts.stale_position;
    let data = pos.try_borrow_data()?;

    // Verify the discriminator matches CdpPosition (sha256("account:CdpPosition")[0..8]).
    const CDP_POSITION_DISC: [u8; 8] = [0x40, 0xfe, 0x87, 0xe6, 0x29, 0x81, 0x26, 0x09];
    require!(data.len() >= 8 && data[..8] == CDP_POSITION_DISC, CdpError::InvalidAccount);

    // Read the owner from bytes 8..40.
    require!(data.len() >= 40, CdpError::InvalidAccount);
    let owner = Pubkey::try_from(&data[8..40]).map_err(|_| error!(CdpError::InvalidAccount))?;

    // Read the nonce from byte 144 (same offset in both old and new struct).
    require!(data.len() >= 145, CdpError::InvalidAccount);
    let nonce = data[144];

    drop(data);

    // Verify the PDA matches the declared seeds.
    let (expected_pda, _bump) = Pubkey::find_program_address(
        &[b"cdp_position", owner.as_ref(), &[nonce]],
        ctx.program_id,
    );
    require!(expected_pda == pos.key(), CdpError::InvalidAccount);

    // Transfer all lamports to authority, zeroing the account.
    let lamports = pos.lamports();
    **pos.try_borrow_mut_lamports()? -= lamports;
    **ctx.accounts.authority.try_borrow_mut_lamports()? += lamports;

    // Zero the data so the runtime reaps the account.
    pos.try_borrow_mut_data()?.fill(0);

    msg!("Closed stale CdpPosition {} (owner: {}, nonce: {})", pos.key(), owner, nonce);
    Ok(())
}

#[derive(Accounts)]
pub struct CloseStalePosition<'info> {
    #[account(
        mut,
        constraint = authority.key() == cdp_config.authority @ CdpError::Unauthorized
    )]
    pub authority: Signer<'info>,

    #[account(seeds = [b"cdp_config"], bump = cdp_config.bump)]
    pub cdp_config: Account<'info, CdpConfig>,

    /// CHECK: Validated manually inside handler (discriminator + PDA seeds).
    #[account(mut, owner = crate::ID)]
    pub stale_position: AccountInfo<'info>,

    /// The per-position borrow rewards — closed here alongside the position.
    #[account(
        mut,
        close = authority,
    )]
    pub borrow_rewards: Box<Account<'info, BorrowRewards>>,

    pub system_program: Program<'info, System>,
}
