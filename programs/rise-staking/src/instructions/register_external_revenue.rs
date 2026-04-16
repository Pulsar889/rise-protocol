use anchor_lang::prelude::*;
use crate::state::{GlobalPool, ProtocolTreasury};
use crate::errors::StakingError;

/// Governance program ID — matches the constant in collect_fees.rs.
const GOVERNANCE_PROGRAM_ID: Pubkey = pubkey!("CtMKhgY5xKiwLB5jmQ44PRF9QsUqXqSbiyVbFsidskHz");

/// Byte offset of `total_verise` (u128) in the serialized GovernanceConfig account.
/// Layout: [8 disc][32 authority][32 rise_mint][16 total_verise]...
const TOTAL_VERISE_OFFSET: usize = 72;

/// Called by the CDP program to register revenue that has already been
/// deposited into treasury_vault. Updates the revenue_index so veRISE
/// holders can claim their share.
///
/// Authorization: signer must be the CDP config PDA registered on GlobalPool
/// via set_cdp_config. Only rise-cdp can produce that PDA signature.
pub fn handler(
    ctx: Context<RegisterExternalRevenue>,
    verise_lamports: u64,
    reserve_lamports: u64,
) -> Result<()> {
    let treasury = &mut ctx.accounts.treasury;

    if verise_lamports > 0 {
        // Per-share accumulator: index += amount * INDEX_SCALE / total_verise.
        // Must match collect_fees.rs so both revenue paths use the same scale.
        // At claim time: claimable = index_delta * user_verise / INDEX_SCALE.
        let gov_data = ctx.accounts.governance_config.try_borrow_data()
            .map_err(|_| StakingError::InvalidGovernanceConfig)?;
        require!(gov_data.len() >= TOTAL_VERISE_OFFSET + 16, StakingError::InvalidGovernanceConfig);
        let total_verise = u128::from_le_bytes(
            gov_data[TOTAL_VERISE_OFFSET..TOTAL_VERISE_OFFSET + 16]
                .try_into()
                .map_err(|_| StakingError::InvalidGovernanceConfig)?
        );
        drop(gov_data);

        if total_verise > 0 {
            let index_increment = (verise_lamports as u128)
                .checked_mul(ProtocolTreasury::INDEX_SCALE)
                .ok_or(StakingError::MathOverflow)?
                .checked_div(total_verise)
                .ok_or(StakingError::MathOverflow)?;

            treasury.revenue_index = treasury
                .revenue_index
                .checked_add(index_increment)
                .ok_or(StakingError::MathOverflow)?;
        }

        treasury.total_distributed = treasury
            .total_distributed
            .checked_add(verise_lamports as u128)
            .ok_or(StakingError::MathOverflow)?;
    }

    if reserve_lamports > 0 {
        treasury.reserve_lamports = treasury
            .reserve_lamports
            .checked_add(reserve_lamports as u128)
            .ok_or(StakingError::MathOverflow)?;
    }

    msg!(
        "External revenue registered: {} veRISE lamports, {} reserve lamports",
        verise_lamports,
        reserve_lamports
    );
    msg!("New revenue index: {}", treasury.revenue_index);

    Ok(())
}

#[derive(Accounts)]
pub struct RegisterExternalRevenue<'info> {
    /// CDP config PDA — must match global_pool.cdp_config_pubkey.
    /// The CDP program signs this CPI with its cdp_config PDA seeds [b"cdp_config"].
    pub cdp_config: Signer<'info>,

    #[account(
        seeds = [b"global_pool"],
        bump = global_pool.bump,
        constraint = cdp_config.key() == global_pool.cdp_config_pubkey @ StakingError::Unauthorized
    )]
    pub global_pool: Account<'info, GlobalPool>,

    #[account(
        mut,
        seeds = [b"protocol_treasury"],
        bump = treasury.bump
    )]
    pub treasury: Account<'info, ProtocolTreasury>,

    /// CHECK: GovernanceConfig PDA from the governance program.
    /// Owner validated against the hardcoded governance program ID.
    /// Read-only — provides total_verise for the per-share revenue accumulator.
    #[account(
        constraint = *governance_config.owner == GOVERNANCE_PROGRAM_ID @ StakingError::InvalidGovernanceConfig
    )]
    pub governance_config: UncheckedAccount<'info>,
}
