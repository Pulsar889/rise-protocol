use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::state::{CdpPosition, CollateralConfig, PaymentConfig};
use crate::errors::CdpError;
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;


pub fn handler(ctx: Context<AddCollateral>, amount: u64) -> Result<()> {
    let position = &mut ctx.accounts.position;

    require!(position.is_open, CdpError::PositionClosed);
    require!(amount > 0, CdpError::ZeroAmount);

    // Track entitlement
    ctx.accounts.collateral_config.total_collateral_entitlements = ctx
        .accounts
        .collateral_config
        .total_collateral_entitlements
        .checked_add(amount)
        .ok_or(CdpError::MathOverflow)?;

    let config = &ctx.accounts.collateral_config;

    // Transfer additional collateral from borrower to vault
    let cpi_ctx = CpiContext::new(
        ctx.accounts.token_program.to_account_info(),
        Transfer {
            from: ctx.accounts.borrower_collateral_account.to_account_info(),
            to: ctx.accounts.collateral_vault.to_account_info(),
            authority: ctx.accounts.borrower.to_account_info(),
        },
    );
    token::transfer(cpi_ctx, amount)?;

    // Update position collateral amount
    position.collateral_amount_original = position
        .collateral_amount_original
        .checked_add(amount)
        .ok_or(CdpError::MathOverflow)?;

    // Get updated collateral price and recompute health factor
    let collateral_usd_price = crate::pyth::get_pyth_price(&ctx.accounts.price_update, &ctx.accounts.collateral_config.pyth_price_feed.to_bytes())?;
    let sol_usd_price = crate::pyth::get_pyth_price(&ctx.accounts.sol_price_update, &ctx.accounts.sol_payment_config.pyth_price_feed.to_bytes())?;

    let token_decimals = ctx.accounts.collateral_mint.decimals;
    let decimal_scale = 10u128.pow(token_decimals as u32);

    let collateral_usd_value = (position.collateral_amount_original as u128)
        .checked_mul(collateral_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(decimal_scale)
        .ok_or(CdpError::MathOverflow)?;

    position.collateral_usd_value = collateral_usd_value;

    let total_owed = position.total_rise_sol_owed().ok_or(CdpError::MathOverflow)?;
    let debt_usd = (total_owed as u128)
        .checked_mul(sol_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(1_000_000_000)
        .ok_or(CdpError::MathOverflow)?;

    position.health_factor = CdpPosition::compute_health_factor(
        collateral_usd_value,
        debt_usd,
        config.liquidation_threshold_bps,
    ).ok_or(CdpError::MathOverflow)?;

    msg!("Added {} collateral tokens", amount);
    msg!("New collateral total: {}", position.collateral_amount_original);
    msg!("New health factor: {}", position.health_factor);

    Ok(())
}


#[derive(Accounts)]
pub struct AddCollateral<'info> {
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
        bump = collateral_config.bump,
        constraint = collateral_config.mint == position.collateral_mint @ CdpError::CollateralNotAccepted
    )]
    pub collateral_config: Account<'info, CollateralConfig>,

    #[account(constraint = collateral_mint.key() == collateral_config.mint @ CdpError::CollateralNotAccepted)]
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

    /// SOL payment config — provides the registered SOL/USD price feed pubkey for validation.
    #[account(
        seeds = [b"payment_config", anchor_lang::solana_program::system_program::ID.as_ref()],
        bump = sol_payment_config.bump,
    )]
    pub sol_payment_config: Box<Account<'info, PaymentConfig>>,

    /// Pyth PriceUpdateV2 for collateral token — feed_id validated inside get_pyth_price.
    pub price_update: Account<'info, PriceUpdateV2>,

    /// Pyth PriceUpdateV2 for SOL/USD — feed_id validated inside get_pyth_price.
    pub sol_price_update: Account<'info, PriceUpdateV2>,

    pub token_program: Program<'info, Token>,
}
