use anchor_lang::prelude::*;
use anchor_lang::system_program;
use crate::state::{GlobalPool, WithdrawalTicket};
use crate::errors::StakingError;

/// Redeems a WithdrawalTicket after the epoch delay has passed.
/// Transfers the locked SOL from pool_vault to the owner and closes the ticket.
pub fn handler(ctx: Context<ClaimUnstake>) -> Result<()> {
    require!(!ctx.accounts.pool.paused, StakingError::PoolPaused);

    let ticket = &ctx.accounts.ticket;
    let current_epoch = Clock::get()?.epoch;

    require!(
        current_epoch >= ticket.claimable_epoch,
        StakingError::UnstakeNotReady
    );

    let sol_amount = ticket.sol_amount;

    // Update pool accounting
    let pool = &mut ctx.accounts.pool;
    pool.pending_withdrawals_lamports = pool
        .pending_withdrawals_lamports
        .checked_sub(sol_amount as u128)
        .ok_or(StakingError::MathOverflow)?;

    // Transfer SOL from pool_vault to user
    let vault_bump = ctx.bumps.pool_vault;
    let seeds = &[b"pool_vault".as_ref(), &[vault_bump]];
    let signer = &[&seeds[..]];

    system_program::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.pool_vault.to_account_info(),
                to: ctx.accounts.user.to_account_info(),
            },
            signer,
        ),
        sol_amount,
    )?;

    msg!("Unstake claimed: {} lamports returned to {}", sol_amount, ctx.accounts.user.key());

    Ok(())
}

#[derive(Accounts)]
pub struct ClaimUnstake<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [b"global_pool"],
        bump = pool.bump
    )]
    pub pool: Account<'info, GlobalPool>,

    #[account(
        mut,
        seeds = [b"withdrawal_ticket", user.key().as_ref(), &ticket.nonce.to_le_bytes()],
        bump = ticket.bump,
        constraint = ticket.owner == user.key(),
        close = user
    )]
    pub ticket: Account<'info, WithdrawalTicket>,

    /// CHECK: SOL vault PDA — signs the transfer out to the user.
    #[account(
        mut,
        seeds = [b"pool_vault"],
        bump
    )]
    pub pool_vault: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}
