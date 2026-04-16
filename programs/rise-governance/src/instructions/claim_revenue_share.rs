use anchor_lang::prelude::*;
use anchor_lang::system_program;
use crate::state::{GovernanceConfig, VeLock};
use crate::errors::GovernanceError;

pub fn handler(ctx: Context<ClaimRevenueShare>) -> Result<()> {
    let current_slot = Clock::get()?.slot;
    let lock = &mut ctx.accounts.lock;

    // Guard: expired locks (current_verise == 0 when slot >= lock_end_slot) cannot claim.
    let current_verise = lock.current_verise(current_slot);
    require!(current_verise > 0, GovernanceError::LockExpired);

    let revenue_index = ctx.accounts.treasury.revenue_index;

    // Calculate claimable amount
    let index_delta = revenue_index
        .saturating_sub(lock.last_revenue_index);

    require!(index_delta > 0, GovernanceError::NoRewardsToClaim);

    // claimable = index_delta * lock.verise_amount / INDEX_SCALE
    //
    // The revenue_index accumulator uses a per-share pattern:
    //   at deposit: index += amount * INDEX_SCALE / total_verise
    //   at claim:   claimable = index_delta * user_verise / INDEX_SCALE
    //
    // Uses the lock's initial (non-decayed) verise_amount so that revenue share is
    // proportional to lock size. The expiry guard above prevents expired locks from claiming.
    let index_scale = rise_staking::state::ProtocolTreasury::INDEX_SCALE;
    let claimable = if index_scale > 0 {
        index_delta
            .checked_mul(lock.verise_amount as u128)
            .ok_or(GovernanceError::MathOverflow)?
            .checked_div(index_scale)
            .ok_or(GovernanceError::MathOverflow)? as u64
    } else {
        0
    };

    require!(claimable > 0, GovernanceError::NoRewardsToClaim);

    // Transfer SOL from verise_vault to user
    let vault_bump = ctx.bumps.verise_vault;
    let seeds = &[b"verise_vault".as_ref(), &[vault_bump]];
    let signer = &[&seeds[..]];

    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.system_program.to_account_info(),
        system_program::Transfer {
            from: ctx.accounts.verise_vault.to_account_info(),
            to: ctx.accounts.user.to_account_info(),
        },
        signer,
    );
    system_program::transfer(cpi_ctx, claimable)?;

    // Update lock state
    lock.last_revenue_index = revenue_index;
    lock.total_revenue_claimed = lock.total_revenue_claimed
        .checked_add(claimable)
        .ok_or(GovernanceError::MathOverflow)?;

    msg!("Revenue claimed: {} lamports", claimable);
    msg!("Total claimed all time: {}", lock.total_revenue_claimed);

    Ok(())
}

#[derive(Accounts)]
pub struct ClaimRevenueShare<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        seeds = [b"governance_config"],
        bump = config.bump
    )]
    pub config: Account<'info, GovernanceConfig>,

    #[account(
        mut,
        seeds = [b"ve_lock", user.key().as_ref(), &[lock.nonce]],
        bump = lock.bump,
        constraint = lock.owner == user.key()
    )]
    pub lock: Account<'info, VeLock>,

    #[account(
        seeds = [b"protocol_treasury"],
        bump,
        seeds::program = rise_staking::ID
    )]
    pub treasury: Account<'info, rise_staking::state::ProtocolTreasury>,

    /// CHECK: Treasury SOL vault — PDA from staking program, bump derived by Anchor.
    #[account(
        mut,
        seeds = [b"verise_vault"],
        bump,
        seeds::program = rise_staking::ID
    )]
    pub verise_vault: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}
