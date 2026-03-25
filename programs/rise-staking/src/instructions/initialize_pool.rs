use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token};
use crate::state::GlobalPool;
use crate::errors::StakingError;

pub fn handler(
    ctx: Context<InitializePool>,
    protocol_fee_bps: u16,
    liquid_buffer_target_bps: u16,
) -> Result<()> {
    // Validate inputs
    require!(protocol_fee_bps <= 10_000, StakingError::InvalidFeeBps);
    require!(
        liquid_buffer_target_bps >= GlobalPool::MIN_LIQUID_BUFFER_BPS
            && liquid_buffer_target_bps <= 10_000,
        StakingError::InvalidBufferBps
    );

    let pool = &mut ctx.accounts.pool;

    pool.authority = ctx.accounts.authority.key();
    pool.rise_sol_mint = ctx.accounts.rise_sol_mint.key();
    pool.total_sol_staked = 0;
    pool.staking_rise_sol_supply = 0;
    pool.exchange_rate = GlobalPool::RATE_SCALE; // 1.0 at genesis
    pool.last_rate_update_epoch = Clock::get()?.epoch;
    pool.liquid_buffer_lamports = 0;
    pool.liquid_buffer_target_bps = liquid_buffer_target_bps;
    pool.protocol_fee_bps = protocol_fee_bps;
    pool.paused = false;
    pool.bump = ctx.bumps.pool;

    msg!("Rise staking pool initialized");
    msg!("Authority: {}", pool.authority);
    msg!("riseSOL mint: {}", pool.rise_sol_mint);
    msg!("Protocol fee: {} bps", protocol_fee_bps);
    msg!("Liquid buffer target: {} bps", liquid_buffer_target_bps);

    Ok(())
}

#[derive(Accounts)]
pub struct InitializePool<'info> {
    /// The authority (governance multisig in production, deployer in dev).
    #[account(mut)]
    pub authority: Signer<'info>,

    /// The global pool state account. Created here.
    #[account(
        init,
        payer = authority,
        space = GlobalPool::SIZE,
        seeds = [b"global_pool"],
        bump
    )]
    pub pool: Account<'info, GlobalPool>,

    /// The riseSOL token mint. Created before this instruction.
    pub rise_sol_mint: Account<'info, Mint>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}
