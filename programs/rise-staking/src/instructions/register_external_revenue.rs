use anchor_lang::prelude::*;
use crate::state::{GlobalPool, ProtocolTreasury};
use crate::errors::StakingError;

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
        treasury.revenue_index = treasury
            .revenue_index
            .checked_add(
                (verise_lamports as u128)
                    .checked_mul(ProtocolTreasury::INDEX_SCALE)
                    .ok_or(StakingError::MathOverflow)?,
            )
            .ok_or(StakingError::MathOverflow)?;

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
}
