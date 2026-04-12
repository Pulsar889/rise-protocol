use anchor_lang::prelude::*;
use anchor_lang::system_program;
use crate::state::{GlobalPool, ProtocolTreasury};
use crate::errors::StakingError;

/// Called by the CDP program via CPI to fund a collateral buyback from the protocol treasury.
///
/// When a borrower fully repays a riseSOL-denominated CDP position and their collateral
/// was previously seized (via `redeem_collateral_for_liquidity`), the protocol owes them
/// the seized tokens back. Since riseSOL repayment burns tokens rather than producing SOL,
/// the treasury covers the buyback cost.
///
/// Transfers `shortfall_sol` lamports from `reserve_vault` to `cdp_wsol_buyback_vault`
/// (a WSOL token account on the CDP side). The CDP program then calls `sync_native` and
/// invokes Jupiter to swap WSOL → collateral tokens → borrower.
///
/// Authorization: signer must be the CDP config PDA registered on GlobalPool via `set_cdp_config`.
pub fn handler(ctx: Context<WithdrawTreasuryForCdpBuyback>, shortfall_sol: u64) -> Result<()> {
    require!(shortfall_sol > 0, StakingError::ZeroAmount);

    // Leave at least rent-exemption in the treasury vault.
    let rent_floor = Rent::get()?.minimum_balance(0);
    let available = ctx.accounts.reserve_vault.lamports().saturating_sub(rent_floor);
    require!(available >= shortfall_sol, StakingError::InsufficientLiquidity);

    let vault_bump = ctx.bumps.reserve_vault;
    let seeds = &[b"reserve_vault".as_ref(), &[vault_bump]];
    let signer = &[&seeds[..]];

    system_program::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.reserve_vault.to_account_info(),
                to:   ctx.accounts.cdp_wsol_buyback_vault.to_account_info(),
            },
            signer,
        ),
        shortfall_sol,
    )?;

    // Keep treasury accounting accurate.
    ctx.accounts.treasury.reserve_lamports =
        ctx.accounts.treasury.reserve_lamports.saturating_sub(shortfall_sol as u128);

    msg!("Treasury buyback: {} lamports → CDP WSOL buyback vault", shortfall_sol);

    Ok(())
}

#[derive(Accounts)]
pub struct WithdrawTreasuryForCdpBuyback<'info> {
    /// CDP config PDA — must match global_pool.cdp_config_pubkey.
    /// The CDP program signs this CPI with seeds [b"cdp_config"].
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

    /// CHECK: PDA verified by seeds; holds native SOL.
    #[account(
        mut,
        seeds = [b"reserve_vault"],
        bump
    )]
    pub reserve_vault: UncheckedAccount<'info>,

    /// CHECK: CDP WSOL buyback vault — receives lamports for Jupiter WSOL wrapping.
    #[account(mut)]
    pub cdp_wsol_buyback_vault: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}
