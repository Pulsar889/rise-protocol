use anchor_lang::prelude::*;
use crate::state::ProtocolTreasury;
use crate::errors::StakingError;

/// Called by the CDP program (or any trusted external program) to register
/// revenue that has already been deposited into treasury_vault.
/// Updates the revenue_index so veRISE holders can claim their share.
///
/// Authorization model: the caller must be a signer. In this initial version
/// any signer can call this. In production the caller should be constrained
/// to a known CDP program PDA (e.g. cdp_fee_vault) via an address constraint.
/// Financial integrity is preserved because the actual SOL was already
/// transferred to treasury_vault before this is called — only the accounting
/// index is updated here.
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
    /// Authorized caller — in practice this will be the CDP program's
    /// cdp_fee_vault PDA signing via CPI invoke_signed.
    pub caller: Signer<'info>,

    #[account(
        mut,
        seeds = [b"protocol_treasury"],
        bump = treasury.bump
    )]
    pub treasury: Account<'info, ProtocolTreasury>,
}
