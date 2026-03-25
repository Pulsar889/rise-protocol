use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount};
use crate::state::GlobalPool;
use crate::errors::StakingError;

/// Called by the CDP program via CPI to mint riseSOL to a borrower.
///
/// The riseSOL mint authority is the GlobalPool PDA, so only the staking
/// program can mint. This instruction gates minting behind the cdp_config
/// PDA signer, meaning only rise-cdp can call it.
///
/// Note: does NOT update staking_rise_sol_supply — CDP-minted riseSOL is
/// tracked separately in cdp_rise_sol_minted on the CDP program side.
pub fn handler(ctx: Context<MintForCdp>, amount: u64) -> Result<()> {
    require!(amount > 0, StakingError::ZeroAmount);

    let pool_bump = ctx.accounts.global_pool.bump;
    let seeds = &[b"global_pool".as_ref(), &[pool_bump]];
    let signer = &[&seeds[..]];

    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        MintTo {
            mint: ctx.accounts.rise_sol_mint.to_account_info(),
            to: ctx.accounts.recipient.to_account_info(),
            authority: ctx.accounts.global_pool.to_account_info(),
        },
        signer,
    );
    token::mint_to(cpi_ctx, amount)?;

    msg!("CDP minted {} riseSOL to borrower", amount);

    Ok(())
}

#[derive(Accounts)]
pub struct MintForCdp<'info> {
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
        address = global_pool.rise_sol_mint
    )]
    pub rise_sol_mint: Account<'info, Mint>,

    /// Borrower's riseSOL token account to mint into.
    #[account(
        mut,
        constraint = recipient.mint == global_pool.rise_sol_mint
    )]
    pub recipient: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}
