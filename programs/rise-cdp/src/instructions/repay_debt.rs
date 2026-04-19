use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token::{TokenAccount, Mint};
use crate::state::{CdpPosition, CollateralConfig, PaymentConfig, CdpConfig, BorrowRewards, BorrowRewardsConfig};
use crate::errors::CdpError;
use rise_staking::state::GlobalPool;
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;

/// Repay all or part of a CDP position's debt using native SOL.
///
/// Payment is split into:
///   - Interest portion  → cdp_fee_vault
///   - Principal portion → pool_vault
///
/// On full repayment with a collateral shortfall, the shortfall-equivalent SOL is
/// diverted to cdp_wsol_buyback_vault. Call claim_collateral afterward.
#[inline(never)]
pub(crate) fn accrue_interest(
    position: &mut CdpPosition,
    cdp_config: &CdpConfig,
    collateral_config: &CollateralConfig,
    staking_supply: u128,
    current_slot: u64,
) -> Result<()> {
    if current_slot > position.last_accrual_slot && position.rise_sol_debt_principal > 0 {
        let slots_elapsed = current_slot
            .checked_sub(position.last_accrual_slot)
            .ok_or(CdpError::MathOverflow)? as u128;

        let ceiling = staking_supply
            .checked_mul(cdp_config.debt_ceiling_multiplier_bps as u128)
            .ok_or(CdpError::MathOverflow)?
            .checked_div(10_000)
            .ok_or(CdpError::MathOverflow)?;

        let utilization_bps: u128 = if ceiling == 0 {
            10_000
        } else {
            (cdp_config.cdp_rise_sol_minted
                .checked_mul(10_000)
                .ok_or(CdpError::MathOverflow)?
                .checked_div(ceiling)
                .ok_or(CdpError::MathOverflow)?)
                .min(10_000)
        };

        let optimal = collateral_config.optimal_utilization_bps as u128;

        let effective_rate_bps: u128 = if utilization_bps <= optimal {
            let slope1_contribution = if optimal == 0 {
                0
            } else {
                (collateral_config.rate_slope1_bps as u128)
                    .checked_mul(utilization_bps)
                    .ok_or(CdpError::MathOverflow)?
                    .checked_div(optimal)
                    .ok_or(CdpError::MathOverflow)?
            };
            (collateral_config.base_rate_bps as u128)
                .checked_add(slope1_contribution)
                .ok_or(CdpError::MathOverflow)?
        } else {
            let excess = utilization_bps
                .checked_sub(optimal)
                .ok_or(CdpError::MathOverflow)?;
            let range = 10_000u128
                .checked_sub(optimal)
                .ok_or(CdpError::MathOverflow)?;
            let slope2_contribution = if range == 0 {
                collateral_config.rate_slope2_bps as u128
            } else {
                (collateral_config.rate_slope2_bps as u128)
                    .checked_mul(excess)
                    .ok_or(CdpError::MathOverflow)?
                    .checked_div(range)
                    .ok_or(CdpError::MathOverflow)?
            };
            (collateral_config.base_rate_bps as u128)
                .checked_add(collateral_config.rate_slope1_bps as u128)
                .ok_or(CdpError::MathOverflow)?
                .checked_add(slope2_contribution)
                .ok_or(CdpError::MathOverflow)?
        };

        let interest = (position.rise_sol_debt_principal as u128)
            .checked_mul(effective_rate_bps)
            .ok_or(CdpError::MathOverflow)?
            .checked_mul(slots_elapsed)
            .ok_or(CdpError::MathOverflow)?
            .checked_div(10_000)
            .ok_or(CdpError::MathOverflow)?
            .checked_div(CollateralConfig::SLOTS_PER_YEAR)
            .ok_or(CdpError::MathOverflow)?;

        let interest_u64 = u64::try_from(interest).map_err(|_| CdpError::MathOverflow)?;

        if interest_u64 > 0 {
            position.interest_accrued = position
                .interest_accrued
                .checked_add(interest_u64)
                .ok_or(CdpError::MathOverflow)?;
            position.last_accrual_slot = current_slot;
        }
    }
    Ok(())
}

pub(crate) struct DebtSettlement {
    pub interest_sol: u64,
    pub principal_sol: u64,
    pub is_fully_repaid: bool,
}

#[inline(never)]
pub(crate) fn settle_repayment(
    position: &mut CdpPosition,
    payment_sol_lamports: u64,
    exchange_rate: u128,
    rate_scale: u128,
    cdp_config: &mut CdpConfig,
    borrow_rewards_config: &mut BorrowRewardsConfig,
    borrow_rewards: &mut BorrowRewards,
) -> Result<DebtSettlement> {
    let payment_rise_sol_u128 = (payment_sol_lamports as u128)
        .checked_mul(rate_scale)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(exchange_rate)
        .ok_or(CdpError::MathOverflow)?;
    let payment_rise_sol = u64::try_from(payment_rise_sol_u128).map_err(|_| CdpError::MathOverflow)?;
    require!(payment_rise_sol > 0, CdpError::ZeroAmount);

    let reward_per_token = borrow_rewards_config.reward_per_token;
    borrow_rewards.settle(reward_per_token, position.rise_sol_debt_principal)?;

    let total_owed = position.total_rise_sol_owed().ok_or(CdpError::MathOverflow)?;
    require!(total_owed > 0, CdpError::ZeroAmount);

    let cleared_rise_sol = payment_rise_sol.min(total_owed);

    let (interest_cleared_rise_sol, principal_cleared_rise_sol) =
        if cleared_rise_sol <= position.interest_accrued {
            (cleared_rise_sol, 0u64)
        } else {
            let remaining = cleared_rise_sol
                .checked_sub(position.interest_accrued)
                .ok_or(CdpError::MathOverflow)?;
            (position.interest_accrued, remaining)
        };

    position.interest_accrued = position
        .interest_accrued
        .checked_sub(interest_cleared_rise_sol)
        .ok_or(CdpError::MathOverflow)?;
    position.rise_sol_debt_principal = position
        .rise_sol_debt_principal
        .checked_sub(principal_cleared_rise_sol)
        .ok_or(CdpError::MathOverflow)?;

    let interest_sol = (interest_cleared_rise_sol as u128)
        .checked_mul(exchange_rate)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(rate_scale)
        .ok_or(CdpError::MathOverflow)? as u64;

    let cleared_sol = (cleared_rise_sol as u128)
        .checked_mul(exchange_rate)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(rate_scale)
        .ok_or(CdpError::MathOverflow)? as u64;

    let principal_sol = cleared_sol.saturating_sub(interest_sol);

    if principal_cleared_rise_sol > 0 {
        cdp_config.cdp_rise_sol_minted = cdp_config
            .cdp_rise_sol_minted
            .saturating_sub(principal_cleared_rise_sol as u128);
        borrow_rewards_config.total_cdp_debt = borrow_rewards_config
            .total_cdp_debt
            .saturating_sub(principal_cleared_rise_sol);
    }

    borrow_rewards.sync_debt(reward_per_token, position.rise_sol_debt_principal)?;

    let is_fully_repaid = position.interest_accrued == 0 && position.rise_sol_debt_principal == 0;

    Ok(DebtSettlement { interest_sol, principal_sol, is_fully_repaid })
}

#[inline(never)]
pub(crate) fn compute_shortfall_divert(
    collateral_amount_original: u64,
    collateral_vault_amount: u64,
    principal_sol: u64,
    price_update: &Account<PriceUpdateV2>,
    sol_price_update: &Account<PriceUpdateV2>,
    collateral_pyth_feed: &[u8; 32],
    sol_pyth_feed: &[u8; 32],
    collateral_decimals: u8,
) -> Result<u64> {
    let available = collateral_vault_amount.min(collateral_amount_original);
    let sf_tokens = collateral_amount_original.saturating_sub(available);
    if sf_tokens == 0 { return Ok(0); }

    let coll_price = crate::pyth::get_pyth_price(price_update, collateral_pyth_feed)?;
    let sol_price  = crate::pyth::get_pyth_price(sol_price_update, sol_pyth_feed)?;
    let dec_scale  = 10u128.pow(collateral_decimals as u32);

    let sf_usd = (sf_tokens as u128)
        .checked_mul(coll_price).ok_or(CdpError::MathOverflow)?
        .checked_div(dec_scale).ok_or(CdpError::MathOverflow)?;

    let sf_sol_raw = sf_usd
        .checked_mul(1_000_000_000u128).ok_or(CdpError::MathOverflow)?
        .checked_div(sol_price).ok_or(CdpError::MathOverflow)? as u64;

    Ok(sf_sol_raw.min(principal_sol))
}

#[inline(never)]
pub(crate) fn route_native_sol<'info>(
    borrower: AccountInfo<'info>,
    cdp_fee_vault: AccountInfo<'info>,
    pool_vault: AccountInfo<'info>,
    cdp_wsol_buyback_vault: AccountInfo<'info>,
    system_program: AccountInfo<'info>,
    interest_sol: u64,
    principal_to_pool: u64,
    shortfall_sol_divert: u64,
) -> Result<()> {
    if interest_sol > 0 {
        system_program::transfer(
            CpiContext::new(system_program.clone(), system_program::Transfer {
                from: borrower.clone(),
                to:   cdp_fee_vault.clone(),
            }),
            interest_sol,
        )?;
    }
    if principal_to_pool > 0 {
        system_program::transfer(
            CpiContext::new(system_program.clone(), system_program::Transfer {
                from: borrower.clone(),
                to:   pool_vault.clone(),
            }),
            principal_to_pool,
        )?;
    }
    if shortfall_sol_divert > 0 {
        system_program::transfer(
            CpiContext::new(system_program.clone(), system_program::Transfer {
                from: borrower.clone(),
                to:   cdp_wsol_buyback_vault.clone(),
            }),
            shortfall_sol_divert,
        )?;
    }
    Ok(())
}

pub fn handler(ctx: Context<RepayDebt>, payment_amount: u64) -> Result<()> {
    require!(payment_amount > 0, CdpError::ZeroAmount);

    let payment_sol_lamports = payment_amount;

    let current_slot = Clock::get()?.slot;
    let staking_supply = ctx.accounts.global_pool.staking_rise_sol_supply;
    accrue_interest(
        &mut ctx.accounts.position,
        &ctx.accounts.cdp_config,
        &ctx.accounts.collateral_config,
        staking_supply,
        current_slot,
    )?;

    let exchange_rate = ctx.accounts.global_pool.exchange_rate;
    let settlement = settle_repayment(
        &mut ctx.accounts.position,
        payment_sol_lamports,
        exchange_rate,
        GlobalPool::RATE_SCALE,
        &mut ctx.accounts.cdp_config,
        &mut ctx.accounts.borrow_rewards_config,
        &mut ctx.accounts.borrow_rewards,
    )?;
    let DebtSettlement { interest_sol, principal_sol, is_fully_repaid } = settlement;

    let shortfall_sol_divert: u64 = if is_fully_repaid {
        compute_shortfall_divert(
            ctx.accounts.position.collateral_amount_original,
            ctx.accounts.collateral_vault.amount,
            principal_sol,
            &ctx.accounts.price_update,
            &ctx.accounts.sol_price_update,
            &ctx.accounts.collateral_config.pyth_price_feed.to_bytes(),
            &ctx.accounts.sol_payment_config.pyth_price_feed.to_bytes(),
            ctx.accounts.collateral_mint.decimals,
        )?
    } else {
        0u64
    };

    let principal_to_pool = principal_sol.saturating_sub(shortfall_sol_divert);

    route_native_sol(
        ctx.accounts.borrower.to_account_info(),
        ctx.accounts.cdp_fee_vault.to_account_info(),
        ctx.accounts.pool_vault.to_account_info(),
        ctx.accounts.cdp_wsol_buyback_vault.to_account_info(),
        ctx.accounts.system_program.to_account_info(),
        interest_sol,
        principal_to_pool,
        shortfall_sol_divert,
    )?;

    if is_fully_repaid {
        ctx.accounts.position.is_open = false;
        ctx.accounts.position.pending_buyback_lamports = shortfall_sol_divert;
        msg!(
            "Position fully repaid. Call claim_collateral to receive collateral. Pending buyback: {}",
            shortfall_sol_divert
        );
    }

    msg!("SOL to fee vault:    {}", interest_sol);
    msg!("SOL to pool backing: {}", principal_to_pool);

    Ok(())
}

#[derive(Accounts)]
pub struct RepayDebt<'info> {
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
        constraint = collateral_config.mint == position.collateral_mint
    )]
    pub collateral_config: Box<Account<'info, CollateralConfig>>,

    #[account(
        seeds = [b"global_pool"],
        seeds::program = rise_staking::ID,
        bump = global_pool.bump
    )]
    pub global_pool: Box<Account<'info, GlobalPool>>,

    #[account(
        mut,
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Box<Account<'info, CdpConfig>>,

    /// CHECK: PDA verified by seeds; holds native SOL.
    #[account(
        mut,
        seeds = [b"cdp_fee_vault"],
        bump
    )]
    pub cdp_fee_vault: UncheckedAccount<'info>,

    /// CHECK: Staking pool SOL vault — receives principal portion.
    #[account(
        mut,
        seeds = [b"pool_vault"],
        seeds::program = rise_staking::ID,
        bump
    )]
    pub pool_vault: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [b"collateral_vault", collateral_config.mint.as_ref()],
        bump,
        constraint = collateral_vault.mint == collateral_config.mint
    )]
    pub collateral_vault: Box<Account<'info, TokenAccount>>,

    #[account(constraint = collateral_mint.key() == collateral_config.mint @ CdpError::CollateralNotAccepted)]
    pub collateral_mint: Box<Account<'info, Mint>>,

    #[account(
        seeds = [b"payment_config", anchor_lang::solana_program::system_program::ID.as_ref()],
        bump = sol_payment_config.bump,
    )]
    pub sol_payment_config: Box<Account<'info, PaymentConfig>>,

    /// Pyth PriceUpdateV2 for collateral token — used for shortfall buyback valuation.
    pub price_update: Box<Account<'info, PriceUpdateV2>>,

    /// Pyth PriceUpdateV2 for SOL/USD — used for shortfall buyback valuation.
    pub sol_price_update: Box<Account<'info, PriceUpdateV2>>,

    #[account(
        mut,
        seeds = [b"cdp_wsol_buyback_vault"],
        bump,
    )]
    pub cdp_wsol_buyback_vault: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [b"borrow_rewards_config"],
        bump = borrow_rewards_config.bump
    )]
    pub borrow_rewards_config: Box<Account<'info, BorrowRewardsConfig>>,

    #[account(
        mut,
        seeds = [b"borrow_rewards", position.key().as_ref()],
        bump = borrow_rewards.bump,
        constraint = borrow_rewards.position == position.key()
    )]
    pub borrow_rewards: Box<Account<'info, BorrowRewards>>,

    pub system_program: Program<'info, System>,
}
