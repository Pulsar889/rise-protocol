use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, Burn, Mint};
use crate::state::{CdpPosition, CollateralConfig, CdpConfig, BorrowRewards, BorrowRewardsConfig};
use crate::errors::CdpError;
use rise_staking::state::GlobalPool;
use rise_staking::program::RiseStaking;


pub fn handler(ctx: Context<ClosePosition>) -> Result<()> {
    let position = &mut ctx.accounts.position;
    let config_mint = ctx.accounts.collateral_config.mint;

    require!(position.is_open, CdpError::PositionClosed);

    // Calculate total riseSOL owed
    let total_owed = position
        .total_rise_sol_owed()
        .ok_or(CdpError::MathOverflow)?;

    require!(total_owed > 0, CdpError::ZeroAmount);

    // Verify borrower is sending enough riseSOL to cover debt
    require!(
        ctx.accounts.borrower_rise_sol_account.amount >= total_owed,
        CdpError::RepaymentExceedsDebt
    );

    // --- Burn riseSOL from borrower ---
    let cpi_ctx = CpiContext::new(
        ctx.accounts.token_program.to_account_info(),
        Burn {
            mint: ctx.accounts.rise_sol_mint.to_account_info(),
            from: ctx.accounts.borrower_rise_sol_account.to_account_info(),
            authority: ctx.accounts.borrower.to_account_info(),
        },
    );
    token::burn(cpi_ctx, total_owed)?;

    // --- Notify staking pool of interest burn so exchange rate adjusts ---
    // Burning interest riseSOL without updating staking_rise_sol_supply would
    // leave the denominator too high, understating the exchange rate permanently.
    let interest_burned = position.interest_accrued;
    if interest_burned > 0 {
        let bump = ctx.accounts.cdp_config.bump;
        let signer_seeds: &[&[&[u8]]] = &[&[b"cdp_config", &[bump]]];
        rise_staking::cpi::notify_rise_sol_burned(
            CpiContext::new_with_signer(
                ctx.accounts.staking_program.to_account_info(),
                rise_staking::cpi::accounts::NotifyRiseSolBurned {
                    cdp_config: ctx.accounts.cdp_config.to_account_info(),
                    global_pool: ctx.accounts.global_pool.to_account_info(),
                },
                signer_seeds,
            ),
            interest_burned,
        )?;
    }

    // --- Decrement entitlement counter ---
    ctx.accounts.collateral_config.total_collateral_entitlements = ctx
        .accounts
        .collateral_config
        .total_collateral_entitlements
        .saturating_sub(position.collateral_amount_original);

    // --- Settle borrow rewards before zeroing debt ---
    // Captures all RISE rewards accrued since the last checkpoint into pending_rewards
    // so they remain claimable via claim_borrow_rewards after the position is closed.
    {
        let reward_per_token = ctx.accounts.borrow_rewards_config.reward_per_token;
        let current_debt = position.rise_sol_debt_principal;
        ctx.accounts.borrow_rewards.settle(reward_per_token, current_debt)?;
        // Sync debt to zero so future settle() calls on the closed position produce 0.
        ctx.accounts.borrow_rewards.sync_debt(reward_per_token, 0)?;
    }

    // --- Decrement global CDP minted counter (principal only) ---
    ctx.accounts.cdp_config.cdp_rise_sol_minted = ctx
        .accounts
        .cdp_config
        .cdp_rise_sol_minted
        .saturating_sub(position.rise_sol_debt_principal as u128);

    // --- Decrement global CDP debt tracker ---
    ctx.accounts.borrow_rewards_config.total_cdp_debt = ctx
        .accounts
        .borrow_rewards_config
        .total_cdp_debt
        .saturating_sub(position.rise_sol_debt_principal);

    // --- Return collateral to borrower ---
    // In production: unstake SOL, convert back to collateral via Jupiter.
    // For v1: return collateral directly from vault.
    let config_mint_ref = config_mint.as_ref();
    let seeds = &[
        b"collateral_vault".as_ref(),
        config_mint_ref,
        &[ctx.bumps.collateral_vault],
    ];
    let signer = &[&seeds[..]];

    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        Transfer {
            from: ctx.accounts.collateral_vault.to_account_info(),
            to: ctx.accounts.borrower_collateral_account.to_account_info(),
            authority: ctx.accounts.collateral_vault.to_account_info(),
        },
        signer,
    );
    token::transfer(cpi_ctx, position.collateral_amount_original)?;

    // --- Close position ---
    position.is_open = false;
    position.rise_sol_debt_principal = 0;
    position.interest_accrued = 0;

    msg!("Position closed");
    msg!("riseSOL burned: {}", total_owed);
    msg!("Collateral returned: {}", position.collateral_amount_original);

    Ok(())
}

#[derive(Accounts)]
pub struct ClosePosition<'info> {
    #[account(mut)]
    pub borrower: Signer<'info>,

    #[account(
        mut,
        seeds = [b"cdp_position", borrower.key().as_ref(), &[position.nonce]],
        bump = position.bump,
        constraint = position.owner == borrower.key(),
        constraint = position.is_open @ CdpError::PositionClosed
    )]
    pub position: Box<Account<'info, CdpPosition>>,

    #[account(
        mut,
        seeds = [b"collateral_config", collateral_config.mint.as_ref()],
        bump = collateral_config.bump,
        constraint = collateral_config.mint == position.collateral_mint @ CdpError::CollateralNotAccepted
    )]
    pub collateral_config: Box<Account<'info, CollateralConfig>>,

    /// The riseSOL mint.
    #[account(
        mut,
        address = global_pool.rise_sol_mint
    )]
    pub rise_sol_mint: Box<Account<'info, Mint>>,

    /// Borrower's riseSOL account to burn from.
    #[account(
        mut,
        constraint = borrower_rise_sol_account.mint == rise_sol_mint.key(),
        constraint = borrower_rise_sol_account.owner == borrower.key()
    )]
    pub borrower_rise_sol_account: Box<Account<'info, TokenAccount>>,

    /// Borrower's collateral account to return tokens to.
    #[account(
        mut,
        constraint = borrower_collateral_account.mint == collateral_config.mint,
        constraint = borrower_collateral_account.owner == borrower.key()
    )]
    pub borrower_collateral_account: Box<Account<'info, TokenAccount>>,

    /// Protocol collateral vault.
    #[account(
        mut,
        seeds = [b"collateral_vault", collateral_config.mint.as_ref()],
        bump,
        constraint = collateral_vault.mint == collateral_config.mint
    )]
    pub collateral_vault: Box<Account<'info, TokenAccount>>,

    /// Global CDP config — cdp_rise_sol_minted decremented; PDA signs notify CPI.
    #[account(
        mut,
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Box<Account<'info, CdpConfig>>,

    /// GlobalPool from staking — updated by notify_rise_sol_burned CPI.
    #[account(
        mut,
        seeds = [b"global_pool"],
        seeds::program = rise_staking::ID,
        bump = global_pool.bump
    )]
    pub global_pool: Box<Account<'info, GlobalPool>>,

    pub staking_program: Program<'info, RiseStaking>,
    pub token_program: Program<'info, Token>,

    #[account(
        mut,
        seeds = [b"borrow_rewards_config"],
        bump = borrow_rewards_config.bump
    )]
    pub borrow_rewards_config: Box<Account<'info, BorrowRewardsConfig>>,

    /// Per-position borrow rewards — settled here so pending RISE is not lost on close.
    /// Remains open after close_position; call claim_borrow_rewards to collect pending RISE.
    #[account(
        mut,
        seeds = [b"borrow_rewards", position.key().as_ref()],
        bump = borrow_rewards.bump,
        constraint = borrow_rewards.position == position.key()
    )]
    pub borrow_rewards: Box<Account<'info, BorrowRewards>>,
}
