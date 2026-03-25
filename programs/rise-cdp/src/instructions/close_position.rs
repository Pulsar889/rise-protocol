use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, Burn, Mint};
use crate::state::{CdpPosition, CollateralConfig};
use crate::errors::CdpError;


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

    // --- Decrement entitlement counter ---
    ctx.accounts.collateral_config.total_collateral_entitlements = ctx
        .accounts
        .collateral_config
        .total_collateral_entitlements
        .saturating_sub(position.collateral_amount_original);

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
    pub position: Account<'info, CdpPosition>,

    #[account(
        mut,
        seeds = [b"collateral_config", collateral_config.mint.as_ref()],
        bump = collateral_config.bump
    )]
    pub collateral_config: Account<'info, CollateralConfig>,

    /// The riseSOL mint.
    #[account(mut)]
    pub rise_sol_mint: Account<'info, Mint>,

    /// Borrower's riseSOL account to burn from.
    #[account(
        mut,
        constraint = borrower_rise_sol_account.mint == rise_sol_mint.key(),
        constraint = borrower_rise_sol_account.owner == borrower.key()
    )]
    pub borrower_rise_sol_account: Account<'info, TokenAccount>,

    /// Borrower's collateral account to return tokens to.
    #[account(
        mut,
        constraint = borrower_collateral_account.mint == collateral_config.mint,
        constraint = borrower_collateral_account.owner == borrower.key()
    )]
    pub borrower_collateral_account: Account<'info, TokenAccount>,

    /// Protocol collateral vault.
    #[account(
        mut,
        seeds = [b"collateral_vault", collateral_config.mint.as_ref()],
        bump,
        constraint = collateral_vault.mint == collateral_config.mint
    )]
    pub collateral_vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}
