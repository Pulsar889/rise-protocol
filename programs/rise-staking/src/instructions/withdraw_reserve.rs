use anchor_lang::prelude::*;
use anchor_lang::system_program;
use crate::state::ProtocolTreasury;
use crate::errors::StakingError;

/// Authority-only: withdraw accumulated reserve SOL from reserve_vault to any destination.
pub fn handler(ctx: Context<WithdrawReserve>, amount: u64) -> Result<()> {
    require!(amount > 0, StakingError::ZeroAmount);
    require!(
        ctx.accounts.treasury.reserve_lamports >= amount as u128,
        StakingError::InsufficientLiquidity
    );

    // Leave rent-exemption in the vault so the account stays alive.
    let rent_floor = Rent::get()?.minimum_balance(0);
    let available = ctx.accounts.reserve_vault.lamports().saturating_sub(rent_floor);
    require!(available >= amount, StakingError::InsufficientLiquidity);

    let vault_bump = ctx.bumps.reserve_vault;
    let seeds = &[b"reserve_vault".as_ref(), &[vault_bump]];
    let signer = &[&seeds[..]];

    system_program::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.reserve_vault.to_account_info(),
                to:   ctx.accounts.destination.to_account_info(),
            },
            signer,
        ),
        amount,
    )?;

    ctx.accounts.treasury.reserve_lamports = ctx.accounts.treasury.reserve_lamports
        .checked_sub(amount as u128)
        .ok_or(StakingError::MathOverflow)?;

    msg!("Reserve withdrawn: {} lamports → {}", amount, ctx.accounts.destination.key());

    Ok(())
}

#[derive(Accounts)]
pub struct WithdrawReserve<'info> {
    #[account(
        constraint = authority.key() == treasury.authority @ StakingError::Unauthorized
    )]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"protocol_treasury"],
        bump = treasury.bump
    )]
    pub treasury: Account<'info, ProtocolTreasury>,

    /// CHECK: PDA verified by seeds; holds native SOL.
    #[account(
        mut,
        seeds = [b"reserve_vault"],
        bump
    )]
    pub reserve_vault: UncheckedAccount<'info>,

    /// CHECK: Destination wallet — authority decides where the funds go.
    #[account(mut)]
    pub destination: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}
