use anchor_lang::prelude::*;
use crate::state::GlobalPool;
use crate::errors::StakingError;

/// One-time migration: reallocates the GlobalPool account to the new size after
/// adding `prev_exchange_rate` and `prev_rate_update_slot` fields.
///
/// Uses raw AccountInfo to avoid Anchor deserializing the still-undersized account.
/// The new 24 bytes are zero-filled, so prev_exchange_rate = 0 and prev_rate_update_slot = 0
/// until the next update_exchange_rate crank runs.
///
/// Call once after deploying the updated program. Authority-only (verified via
/// the authority field at offset 8 inside the account data).
pub fn handler(ctx: Context<MigrateGlobalPool>) -> Result<()> {
    let pool_info = &ctx.accounts.pool;
    let data = pool_info.try_borrow_data()?;

    // Verify account discriminator matches GlobalPool.
    require!(data[..8] == *GlobalPool::DISCRIMINATOR, StakingError::Unauthorized);

    let authority_bytes = &data[8..40]; // Pubkey at offset 8
    require!(
        authority_bytes == ctx.accounts.authority.key().as_ref(),
        StakingError::Unauthorized
    );

    let old_len = data.len();
    let new_len = GlobalPool::SIZE;
    require!(old_len < new_len, StakingError::AlreadyMigrated);

    drop(data);

    // Realloc via AccountInfo::realloc
    pool_info.realloc(new_len, true)?;

    // Fund rent if needed
    let rent = Rent::get()?;
    let required_lamports = rent.minimum_balance(new_len);
    let current_lamports = pool_info.lamports();
    if current_lamports < required_lamports {
        let diff = required_lamports - current_lamports;
        anchor_lang::system_program::transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                anchor_lang::system_program::Transfer {
                    from: ctx.accounts.authority.to_account_info(),
                    to: pool_info.clone(),
                },
            ),
            diff,
        )?;
    }

    msg!("GlobalPool reallocated: {} → {} bytes", old_len, new_len);
    msg!("prev_exchange_rate and prev_rate_update_slot initialized to 0");
    Ok(())
}

#[derive(Accounts)]
pub struct MigrateGlobalPool<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: Validated manually inside handler (discriminator + authority check).
    #[account(
        mut,
        seeds = [b"global_pool"],
        bump,
    )]
    pub pool: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}
