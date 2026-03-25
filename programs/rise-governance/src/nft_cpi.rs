/// Raw CPI helpers for the Metaplex Token Metadata program.
///
/// We talk to the Token Metadata program directly (no mpl-token-metadata crate)
/// to avoid Solana SDK version conflicts.  The instruction indices and Borsh
/// layout match the on-chain program deployed at
/// `metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s`.
use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    instruction::{AccountMeta, Instruction},
    program::invoke,
    program::invoke_signed,
};

/// Metaplex Token Metadata program ID.
pub const TOKEN_METADATA_PROGRAM_ID: Pubkey = Pubkey::from_str_const(
    "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s",
);

/// Derive the metadata PDA for a given mint.
pub fn metadata_pda(mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[
            b"metadata",
            TOKEN_METADATA_PROGRAM_ID.as_ref(),
            mint.as_ref(),
        ],
        &TOKEN_METADATA_PROGRAM_ID,
    )
    .0
}

// ── Borsh helpers ─────────────────────────────────────────────────────────────

fn push_u16_le(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn push_string(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(bytes);
}

// ── CreateMetadataAccountV3 (instruction index 33) ───────────────────────────
//
// DataV2 layout (Borsh):
//   name: String, symbol: String, uri: String,
//   seller_fee_basis_points: u16,
//   creators: Option<Vec<Creator>>  → None → [0x00]
//   collection: Option<Collection>  → None → [0x00]
//   uses: Option<Uses>              → None → [0x00]
//
// CreateMetadataAccountArgsV3 layout:
//   data: DataV2, is_mutable: bool,
//   collection_details: Option<CollectionDetails> → None → [0x00]
//
// Required accounts (in order):
//   0. metadata       writable, non-signer  (PDA owned by token-metadata program)
//   1. mint           read-only
//   2. mint_authority signer
//   3. payer          writable, signer
//   4. update_authority non-signer
//   5. system_program
//   6. rent           sysvar

pub fn create_metadata_v3<'info>(
    metadata: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    mint_authority: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    update_authority: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    rent: &AccountInfo<'info>,
    token_metadata_program: &AccountInfo<'info>,
    name: &str,
    symbol: &str,
    uri: &str,
    signer_seeds: Option<&[&[&[u8]]]>,
) -> Result<()> {
    let mut data: Vec<u8> = Vec::with_capacity(128);
    data.push(33); // CreateMetadataAccountV3 variant index

    // DataV2
    push_string(&mut data, name);
    push_string(&mut data, symbol);
    push_string(&mut data, uri);
    push_u16_le(&mut data, 0); // seller_fee_basis_points
    data.push(0); // creators: None
    data.push(0); // collection: None
    data.push(0); // uses: None

    // CreateMetadataAccountArgsV3 continuation
    data.push(1); // is_mutable: true
    data.push(0); // collection_details: None

    let accounts = vec![
        AccountMeta::new(metadata.key(), false),
        AccountMeta::new_readonly(mint.key(), false),
        AccountMeta::new_readonly(mint_authority.key(), true),
        AccountMeta::new(payer.key(), true),
        AccountMeta::new_readonly(update_authority.key(), false),
        AccountMeta::new_readonly(system_program.key(), false),
        AccountMeta::new_readonly(rent.key(), false),
    ];

    let ix = Instruction {
        program_id: TOKEN_METADATA_PROGRAM_ID,
        accounts,
        data,
    };

    let account_infos = [
        metadata.clone(),
        mint.clone(),
        mint_authority.clone(),
        payer.clone(),
        update_authority.clone(),
        system_program.clone(),
        rent.clone(),
        token_metadata_program.clone(),
    ];

    if let Some(seeds) = signer_seeds {
        invoke_signed(&ix, &account_infos, seeds)?;
    } else {
        invoke(&ix, &account_infos)?;
    }

    Ok(())
}

// ── BurnNft (instruction index 29) ───────────────────────────────────────────
//
// Burns the NFT: closes the token account, and optionally closes metadata +
// edition accounts.  Required accounts (in order):
//   0. metadata         writable
//   1. owner            writable, signer
//   2. mint             writable
//   3. token_account    writable
//   4. master_edition   writable  (pass metadata PDA again if no edition)
//   5. spl_token_program
//   (6. collection_metadata – optional; omit if no collection)
//
// For positions without a master edition we use a simple token burn instead
// (see `burn_nft_token`).

/// Burn 1 token from the owner's token account using spl-token directly.
/// Works only when the token account is NOT frozen (i.e. no master edition
/// was created).
pub fn burn_nft_token<'info>(
    token_program: &AccountInfo<'info>,
    token_account: &AccountInfo<'info>,
    mint: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
) -> Result<()> {
    anchor_spl::token::burn(
        CpiContext::new(
            token_program.clone(),
            anchor_spl::token::Burn {
                mint: mint.clone(),
                from: token_account.clone(),
                authority: authority.clone(),
            },
        ),
        1,
    )?;
    Ok(())
}
