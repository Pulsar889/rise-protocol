use anchor_lang::prelude::*;
use crate::errors::GovernanceError;

/// One-time migration: reallocates governance_config from 147 → 155 bytes
/// to accommodate the new `active_proposal_count` field added between
/// `lock_count` and `bump`.
///
/// Old layout (147 bytes):
///   [0,8)   discriminator
///   [8,40)  authority
///   [40,72) rise_mint
///   [72,88) total_verise  (u128)
///   [88,96) min_lock_slots
///   [96,104) max_lock_slots
///   [104,112) proposal_threshold
///   [112,120) voting_period_slots
///   [120,128) timelock_slots
///   [128,130) quorum_bps
///   [130,138) proposal_count
///   [138,146) lock_count
///   [146]   bump
///
/// New layout (155 bytes): inserts active_proposal_count at [146,154), bump → [154].
pub fn handler(ctx: Context<MigrateGovernanceConfig>) -> Result<()> {
    let config_info = &ctx.accounts.config;

    // Verify authority matches what is stored in the account (bytes 8..40).
    {
        let data = config_info.try_borrow_data()?;
        require!(data.len() == 147, GovernanceError::InvalidConfig);
        let stored_authority = Pubkey::try_from(&data[8..40])
            .map_err(|_| error!(GovernanceError::InvalidConfig))?;
        require!(
            stored_authority == ctx.accounts.authority.key(),
            GovernanceError::Unauthorized
        );
    }

    // Save the bump byte before realloc changes the slice.
    let old_bump: u8 = {
        let data = config_info.try_borrow_data()?;
        data[146]
    };

    // Realloc: +8 bytes, lamports topped up by payer.
    config_info.resize(155)?;

    // Write active_proposal_count = 0 at [146,154), bump at [154].
    {
        let mut data = config_info.try_borrow_mut_data()?;
        data[146..154].copy_from_slice(&0u64.to_le_bytes());
        data[154] = old_bump;
    }

    // Top up rent exemption for the extra 8 bytes via system_program CPI.
    let rent = Rent::get()?;
    let extra_lamports = rent.minimum_balance(155)
        .saturating_sub(rent.minimum_balance(147));

    if extra_lamports > 0 {
        anchor_lang::system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                anchor_lang::system_program::Transfer {
                    from: ctx.accounts.payer.to_account_info(),
                    to: ctx.accounts.config.to_account_info(),
                },
            ),
            extra_lamports,
        )?;
    }

    msg!("governance_config migrated: 147 → 155 bytes, active_proposal_count = 0");

    Ok(())
}

#[derive(Accounts)]
pub struct MigrateGovernanceConfig<'info> {
    /// Must match the authority stored in the account.
    #[account(mut)]
    pub authority: Signer<'info>,

    /// Pays rent top-up (8 extra bytes).
    #[account(mut)]
    pub payer: Signer<'info>,

    /// CHECK: verified by seeds; deserialized manually because the account
    /// is still in the old 147-byte layout.
    #[account(
        mut,
        seeds = [b"governance_config"],
        bump
    )]
    pub config: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}
