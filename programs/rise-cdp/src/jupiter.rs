//! Thin Jupiter v6 CPI wrapper.
//!
//! `jupiter-cpi` is incompatible with anchor 0.32 due to an `anchor-gen` dependency
//! conflict, so we invoke Jupiter via raw `invoke_signed` instead.
//!
//! # Route plan encoding
//!
//! All instructions that call Jupiter accept a `route_plan_data: Vec<u8>` parameter.
//! This must be the **Borsh-serialized `Vec<RoutePlanStep>`** exactly as Jupiter v6 defines it
//! (including the 4-byte element count prefix).
//!
//! **Frontend side** — build `route_plan_data` like this:
//! ```typescript
//! import { createJupiterApiClient } from "@jup-ag/api";
//! import { BorshCoder } from "@coral-xyz/anchor";
//! import jupiterIdl from "./idls/jupiter_v6.json";
//!
//! const { routePlan } = await jupiterApi.quoteGet({ ... });
//! const coder = new BorshCoder(jupiterIdl as any);
//! const routePlanData = Buffer.from(coder.types.encode("RoutePlanStep[]", routePlan));
//! ```
//! Or use `@jup-ag/api`'s `serializeRoutePlan(routePlan)` utility if available.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    instruction::{AccountMeta, Instruction},
    program::invoke_signed,
};

/// Jupiter v6 program ID (same on mainnet and devnet).
pub const PROGRAM_ID: Pubkey = pubkey!("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4");

/// Instruction discriminator for Jupiter v6 `sharedAccountsRoute`.
/// = sha256("global:shared_accounts_route")[..8]
const DISC: [u8; 8] = [193, 32, 155, 51, 65, 214, 156, 129];

/// Invoke Jupiter v6 `sharedAccountsRoute` via raw CPI.
///
/// `route_plan_data` is the Borsh-serialized `Vec<RoutePlanStep>` bytes (with length prefix)
/// obtained from the Jupiter quote API. `in_amount` is the exact token amount being swapped.
///
/// For PDA-owned source accounts, pass the PDA's signer seeds in `signer_seeds`.
/// For wallet-owned source accounts (borrower), pass an empty slice `&[]`.
#[allow(clippy::too_many_arguments)]
pub fn shared_accounts_route<'info>(
    jupiter_program:      &AccountInfo<'info>,
    program_authority:    &AccountInfo<'info>,
    user_transfer_auth:   &AccountInfo<'info>,
    source_token_account: &AccountInfo<'info>,
    prog_source_token:    &AccountInfo<'info>,
    prog_dest_token:      &AccountInfo<'info>,
    dest_token_account:   &AccountInfo<'info>,
    source_mint:          &AccountInfo<'info>,
    dest_mint:            &AccountInfo<'info>,
    event_authority:      &AccountInfo<'info>,
    token_program:        &AccountInfo<'info>,
    route_plan_data:      &[u8],
    in_amount:            u64,
    quoted_out_amount:    u64,
    slippage_bps:         u16,
    signer_seeds:         &[&[&[u8]]],
) -> Result<()> {
    // Build instruction data: discriminator + id(0) + route_plan + amounts + slippage + fee(0)
    let mut data = Vec::with_capacity(8 + 1 + route_plan_data.len() + 8 + 8 + 2 + 1);
    data.extend_from_slice(&DISC);
    data.push(0u8);                                      // id
    data.extend_from_slice(route_plan_data);             // pre-serialized Vec<RoutePlanStep>
    data.extend_from_slice(&in_amount.to_le_bytes());
    data.extend_from_slice(&quoted_out_amount.to_le_bytes());
    data.extend_from_slice(&slippage_bps.to_le_bytes());
    data.push(0u8);                                      // platform_fee_bps

    // Account ordering matches Jupiter v6 SharedAccountsRoute (no optional accounts).
    let accounts = vec![
        AccountMeta::new_readonly(*token_program.key,       false),
        AccountMeta::new_readonly(*program_authority.key,   false),
        AccountMeta::new_readonly(*user_transfer_auth.key,  true),  // virtual signer via invoke_signed
        AccountMeta::new(*source_token_account.key,         false),
        AccountMeta::new(*prog_source_token.key,            false),
        AccountMeta::new(*prog_dest_token.key,              false),
        AccountMeta::new(*dest_token_account.key,           false),
        AccountMeta::new(*source_mint.key,                  false),
        AccountMeta::new(*dest_mint.key,                    false),
        AccountMeta::new_readonly(*event_authority.key,     false),
        AccountMeta::new_readonly(*jupiter_program.key,     false),
    ];

    let ix = Instruction { program_id: PROGRAM_ID, accounts, data };

    invoke_signed(
        &ix,
        &[
            token_program.clone(),
            program_authority.clone(),
            user_transfer_auth.clone(),
            source_token_account.clone(),
            prog_source_token.clone(),
            prog_dest_token.clone(),
            dest_token_account.clone(),
            source_mint.clone(),
            dest_mint.clone(),
            event_authority.clone(),
            jupiter_program.clone(),
        ],
        signer_seeds,
    )?;

    Ok(())
}
