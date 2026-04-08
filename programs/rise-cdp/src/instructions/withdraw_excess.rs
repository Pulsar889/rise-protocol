use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::state::{CdpPosition, CollateralConfig};
use crate::errors::CdpError;

pub fn handler(ctx: Context<WithdrawExcess>, amount: u64) -> Result<()> {
    let position = &mut ctx.accounts.position;
    let config = &ctx.accounts.collateral_config;

    require!(position.is_open, CdpError::PositionClosed);
    require!(amount > 0, CdpError::ZeroAmount);

    // Get current prices
    let collateral_usd_price = crate::pyth::get_pyth_price(&ctx.accounts.pyth_price_feed)?;
    let sol_usd_price = crate::pyth::get_pyth_price(&ctx.accounts.sol_price_feed)?;

    let token_decimals = ctx.accounts.collateral_mint.decimals;
    let decimal_scale = 10u128.pow(token_decimals as u32);

    // Current collateral USD value
    let collateral_usd_value = (position.collateral_amount_original as u128)
        .checked_mul(collateral_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(decimal_scale)
        .ok_or(CdpError::MathOverflow)?;

    // Current debt USD value
    let total_owed = position.total_rise_sol_owed().ok_or(CdpError::MathOverflow)?;
    let debt_usd = (total_owed as u128)
        .checked_mul(sol_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(1_000_000_000)
        .ok_or(CdpError::MathOverflow)?;

    // Required collateral at safe withdrawal LTV (80% of max LTV)
    let safe_ltv_bps = (config.max_ltv_bps as u32)
        .checked_mul(80)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(100)
        .ok_or(CdpError::MathOverflow)? as u128;

    let required_collateral_usd = debt_usd
        .checked_mul(10_000)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(safe_ltv_bps)
        .ok_or(CdpError::MathOverflow)?;

    // Excess USD value
    require!(
        collateral_usd_value > required_collateral_usd,
        CdpError::InsufficientExcess
    );

    let excess_usd = collateral_usd_value
        .checked_sub(required_collateral_usd)
        .ok_or(CdpError::MathOverflow)?;

    // Convert requested amount to USD
    let requested_usd = (amount as u128)
        .checked_mul(collateral_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(decimal_scale)
        .ok_or(CdpError::MathOverflow)?;

    require!(requested_usd <= excess_usd, CdpError::InsufficientExcess);

    // Transfer excess collateral immediately to borrower
    let config_mint_ref = config.mint.as_ref();
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
    token::transfer(cpi_ctx, amount)?;

    position.collateral_amount_original = position
        .collateral_amount_original
        .checked_sub(amount)
        .ok_or(CdpError::MathOverflow)?;

    ctx.accounts.collateral_config.total_collateral_entitlements = ctx
        .accounts
        .collateral_config
        .total_collateral_entitlements
        .saturating_sub(amount);

    // Update collateral USD value
    position.collateral_usd_value = (position.collateral_amount_original as u128)
        .checked_mul(collateral_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(decimal_scale)
        .ok_or(CdpError::MathOverflow)?;

    msg!("Excess collateral withdrawn: {} tokens", amount);

    Ok(())
}


#[derive(Accounts)]
pub struct WithdrawExcess<'info> {
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

    pub collateral_mint: Account<'info, anchor_spl::token::Mint>,

    #[account(
        mut,
        constraint = borrower_collateral_account.mint == collateral_config.mint,
        constraint = borrower_collateral_account.owner == borrower.key()
    )]
    pub borrower_collateral_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [b"collateral_vault", collateral_config.mint.as_ref()],
        bump,
        constraint = collateral_vault.mint == collateral_config.mint
    )]
    pub collateral_vault: Account<'info, TokenAccount>,

    /// CHECK: Pyth price feed for collateral.
    pub pyth_price_feed: AccountInfo<'info>,

    /// CHECK: Pyth price feed for SOL.
    pub sol_price_feed: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}
