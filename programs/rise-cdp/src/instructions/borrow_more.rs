use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, Token, TokenAccount};
use crate::state::{CdpPosition, CollateralConfig, CdpConfig, BorrowRewards, BorrowRewardsConfig, PaymentConfig};
use crate::errors::CdpError;
use rise_staking::state::GlobalPool;
use rise_staking::program::RiseStaking;
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;

/// Mint additional riseSOL against an existing open position.
///
/// Checks that the new total debt (existing principal + interest + additional)
/// does not exceed the position's max LTV. If healthy, increases
/// `rise_sol_debt_principal` and updates the health factor.
///
/// NOTE: In production the CDP program would CPI to rise-staking to mint
/// actual riseSOL tokens to the borrower, since riseSOL mint authority belongs to
/// the GlobalPool PDA. Until that CPI is added, debt is tracked on-chain but
/// no token is transferred to the borrower's wallet.
pub fn handler(ctx: Context<BorrowMore>, additional_rise_sol: u64) -> Result<()> {
    require!(additional_rise_sol > 0, CdpError::ZeroAmount);

    let position = &mut ctx.accounts.position;
    let config = &ctx.accounts.collateral_config;

    // ── LTV check with new total debt ────────────────────────────────────────
    // Use fresh oracle prices — position.collateral_usd_value may be stale
    // if the collateral price has moved since the position was opened or last topped up.
    let collateral_usd_price = crate::pyth::get_pyth_price(&ctx.accounts.price_update, &ctx.accounts.collateral_config.pyth_price_feed.to_bytes())?;
    let sol_usd_price = crate::pyth::get_pyth_price(&ctx.accounts.sol_price_update, &ctx.accounts.sol_payment_config.pyth_price_feed.to_bytes())?;

    let token_decimals = ctx.accounts.collateral_mint.decimals;
    let decimal_scale = 10u128.pow(token_decimals as u32);

    // Recompute collateral USD value at current market price.
    let fresh_collateral_usd = (position.collateral_amount_original as u128)
        .checked_mul(collateral_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(decimal_scale)
        .ok_or(CdpError::MathOverflow)?;

    // Persist the refreshed value so subsequent checks are also accurate.
    position.collateral_usd_value = fresh_collateral_usd;

    let exchange_rate = ctx.accounts.global_pool.exchange_rate;
    let rate_scale = GlobalPool::RATE_SCALE;

    let current_debt_rise_sol = position.total_rise_sol_owed().ok_or(CdpError::MathOverflow)?;

    let new_total_rise_sol = current_debt_rise_sol
        .checked_add(additional_rise_sol)
        .ok_or(CdpError::MathOverflow)?;

    // new_debt_sol = new_total_rise_sol * exchange_rate / RATE_SCALE (lamports)
    let new_debt_sol = (new_total_rise_sol as u128)
        .checked_mul(exchange_rate)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(rate_scale)
        .ok_or(CdpError::MathOverflow)?;

    // new_debt_usd = new_debt_sol (lamports) * sol_usd_price / 1e9
    let new_debt_usd = new_debt_sol
        .checked_mul(sol_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(1_000_000_000)
        .ok_or(CdpError::MathOverflow)?;

    // max_debt_usd = fresh_collateral_usd * max_ltv_bps / 10_000
    let max_debt_usd = fresh_collateral_usd
        .checked_mul(config.max_ltv_bps as u128)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(10_000)
        .ok_or(CdpError::MathOverflow)?;

    require!(new_debt_usd <= max_debt_usd, CdpError::ExceedsMaxLtv);

    // ── Debt ceiling check ────────────────────────────────────────────────────
    let cdp_config = &mut ctx.accounts.cdp_config;
    let staking_supply = ctx.accounts.global_pool.staking_rise_sol_supply;
    let ceiling = staking_supply
        .checked_mul(cdp_config.debt_ceiling_multiplier_bps as u128)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(10_000)
        .ok_or(CdpError::MathOverflow)?;

    let new_minted = cdp_config.cdp_rise_sol_minted
        .checked_add(additional_rise_sol as u128)
        .ok_or(CdpError::MathOverflow)?;

    require!(new_minted <= ceiling, CdpError::DebtCeilingExceeded);

    // Single-loan cap: total position principal may not exceed 5% of the debt ceiling
    let single_loan_cap = ceiling
        .checked_mul(CdpConfig::MAX_SINGLE_LOAN_BPS)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(10_000)
        .ok_or(CdpError::MathOverflow)?;

    let new_position_principal = position.rise_sol_debt_principal
        .checked_add(additional_rise_sol)
        .ok_or(CdpError::MathOverflow)?;

    require!(
        new_position_principal as u128 <= single_loan_cap,
        CdpError::ExceedsSingleLoanCap
    );

    cdp_config.cdp_rise_sol_minted = new_minted;

    // ── Settle borrow rewards before changing debt ────────────────────────────
    {
        let reward_per_token = ctx.accounts.borrow_rewards_config.reward_per_token;
        let current_debt = position.rise_sol_debt_principal;
        ctx.accounts.borrow_rewards.settle(reward_per_token, current_debt)?;
    }

    // ── Update position ───────────────────────────────────────────────────────
    position.rise_sol_debt_principal = position
        .rise_sol_debt_principal
        .checked_add(additional_rise_sol)
        .ok_or(CdpError::MathOverflow)?;

    position.health_factor = CdpPosition::compute_health_factor(
        position.collateral_usd_value,
        new_debt_usd,
        config.liquidation_threshold_bps,
    )
    .ok_or(CdpError::MathOverflow)?;

    // ── Sync reward_debt to reflect new debt ──────────────────────────────────
    {
        let reward_per_token = ctx.accounts.borrow_rewards_config.reward_per_token;
        let new_principal = position.rise_sol_debt_principal;
        ctx.accounts.borrow_rewards.sync_debt(reward_per_token, new_principal)?;
        ctx.accounts.borrow_rewards_config.total_cdp_debt = ctx
            .accounts.borrow_rewards_config.total_cdp_debt
            .checked_add(additional_rise_sol)
            .ok_or(CdpError::MathOverflow)?;
    }

    // ── Mint additional riseSOL to borrower via CPI to staking program ──────
    let cdp_config_bump = ctx.accounts.cdp_config.bump;
    let signer_seeds: &[&[&[u8]]] = &[&[b"cdp_config", &[cdp_config_bump]]];

    rise_staking::cpi::mint_for_cdp(
        CpiContext::new_with_signer(
            ctx.accounts.staking_program.to_account_info(),
            rise_staking::cpi::accounts::MintForCdp {
                cdp_config: ctx.accounts.cdp_config.to_account_info(),
                global_pool: ctx.accounts.global_pool.to_account_info(),
                rise_sol_mint: ctx.accounts.rise_sol_mint.to_account_info(),
                recipient: ctx.accounts.borrower_rise_sol_account.to_account_info(),
                token_program: ctx.accounts.token_program.to_account_info(),
            },
            signer_seeds,
        ),
        additional_rise_sol,
    )?;

    msg!("Additional riseSOL debt recorded: {}", additional_rise_sol);
    msg!("New rise_sol_debt_principal:       {}", position.rise_sol_debt_principal);
    msg!("New health factor:             {}", position.health_factor);

    Ok(())
}


#[derive(Accounts)]
pub struct BorrowMore<'info> {
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
        seeds = [b"collateral_config", collateral_config.mint.as_ref()],
        bump = collateral_config.bump,
        constraint = collateral_config.mint == position.collateral_mint
    )]
    pub collateral_config: Box<Account<'info, CollateralConfig>>,

    /// GlobalPool from the staking program — read for exchange rate and staking supply.
    #[account(
        seeds = [b"global_pool"],
        seeds::program = rise_staking::ID,
        bump = global_pool.bump
    )]
    pub global_pool: Box<Account<'info, GlobalPool>>,

    /// Global CDP config — tracks total CDP riseSOL minted and debt ceiling.
    #[account(
        mut,
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Box<Account<'info, CdpConfig>>,

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

    /// Collateral mint — needed for decimal scaling when recomputing collateral USD value.
    #[account(constraint = collateral_mint.key() == collateral_config.mint @ CdpError::CollateralNotAccepted)]
    pub collateral_mint: Box<Account<'info, Mint>>,

    /// The riseSOL mint — needed for the mint_for_cdp CPI.
    #[account(
        mut,
        address = global_pool.rise_sol_mint
    )]
    pub rise_sol_mint: Box<Account<'info, Mint>>,

    /// Borrower's riseSOL token account to receive minted tokens.
    #[account(
        mut,
        constraint = borrower_rise_sol_account.mint == global_pool.rise_sol_mint,
        constraint = borrower_rise_sol_account.owner == borrower.key()
    )]
    pub borrower_rise_sol_account: Box<Account<'info, TokenAccount>>,

    pub staking_program: Program<'info, RiseStaking>,
    pub token_program: Program<'info, Token>,

    /// Global borrow rewards config — updated with new total_cdp_debt.
    #[account(
        mut,
        seeds = [b"borrow_rewards_config"],
        bump = borrow_rewards_config.bump
    )]
    pub borrow_rewards_config: Box<Account<'info, BorrowRewardsConfig>>,

    /// Per-position borrow rewards — settled before debt increases.
    #[account(
        mut,
        seeds = [b"borrow_rewards", position.key().as_ref()],
        bump = borrow_rewards.bump,
        constraint = borrow_rewards.position == position.key()
    )]
    pub borrow_rewards: Box<Account<'info, BorrowRewards>>,
}
