use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount};
use crate::state::GlobalPool;
use crate::errors::StakingError;

pub fn handler(ctx: Context<StakeSol>, lamports: u64) -> Result<()> {
    require!(!ctx.accounts.pool.paused, StakingError::PoolPaused);
    require!(lamports > 0, StakingError::ZeroAmount);

    // Grab bump before mutable borrow
    let pool_bump = ctx.accounts.pool.bump;

    // Calculate riseSOL to mint before taking mutable reference
    let rise_sol_to_mint = ctx.accounts.pool
        .sol_to_rise_sol(lamports)
        .ok_or(StakingError::MathOverflow)?;

    require!(rise_sol_to_mint > 0, StakingError::ZeroAmount);

    // Transfer SOL from user to pool vault
    let cpi_ctx = CpiContext::new(
        ctx.accounts.system_program.to_account_info(),
        system_program::Transfer {
            from: ctx.accounts.user.to_account_info(),
            to: ctx.accounts.pool_vault.to_account_info(),
        },
    );
    system_program::transfer(cpi_ctx, lamports)?;

    // Now take mutable reference to update state
    {
        let pool = &mut ctx.accounts.pool;

        pool.total_sol_staked = pool
            .total_sol_staked
            .checked_add(lamports as u128)
            .ok_or(StakingError::MathOverflow)?;

        pool.staking_rise_sol_supply = pool
            .staking_rise_sol_supply
            .checked_add(rise_sol_to_mint as u128)
            .ok_or(StakingError::MathOverflow)?;

        pool.liquid_buffer_lamports = pool
            .liquid_buffer_lamports
            .checked_add(lamports as u128)
            .ok_or(StakingError::MathOverflow)?;
    } // mutable borrow ends here

    // Mint riseSOL to user
    let seeds = &[b"global_pool".as_ref(), &[pool_bump]];
    let signer = &[&seeds[..]];

    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        MintTo {
            mint: ctx.accounts.rise_sol_mint.to_account_info(),
            to: ctx.accounts.user_rise_sol_account.to_account_info(),
            authority: ctx.accounts.pool.to_account_info(),
        },
        signer,
    );
    token::mint_to(cpi_ctx, rise_sol_to_mint)?;

    msg!("Staked {} lamports for {} riseSOL", lamports, rise_sol_to_mint);
    msg!("Total SOL staked: {}", ctx.accounts.pool.total_sol_staked);
    msg!("Total riseSOL supply: {}", ctx.accounts.pool.staking_rise_sol_supply);

    Ok(())
}

#[derive(Accounts)]
pub struct StakeSol<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [b"global_pool"],
        bump = pool.bump
    )]
    pub pool: Account<'info, GlobalPool>,

    /// CHECK: This is a system account PDA used as a SOL vault.
    #[account(
        mut,
        seeds = [b"pool_vault"],
        bump
    )]
    pub pool_vault: UncheckedAccount<'info>,

    #[account(
        mut,
        address = pool.rise_sol_mint
    )]
    pub rise_sol_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = user_rise_sol_account.mint == pool.rise_sol_mint,
        constraint = user_rise_sol_account.owner == user.key()
    )]
    pub user_rise_sol_account: Account<'info, TokenAccount>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}
