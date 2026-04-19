use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token::{self, Token, TokenAccount, Mint, CloseAccount};
use crate::state::{CdpPosition, CollateralConfig, PaymentConfig, CdpConfig, BorrowRewards, BorrowRewardsConfig};
use crate::errors::CdpError;
use rise_staking::state::GlobalPool;
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;
use super::repay_debt::{accrue_interest, settle_repayment, compute_shortfall_divert, DebtSettlement};

/// Repay all or part of a CDP position's debt using an SPL token (USDC, USDT, wBTC, wETH).
/// The payment token is swapped → WSOL → SOL via Jupiter v6 on-chain.
/// The actual swap output is used as the payment value (no oracle dependency for the swap amount).
///
/// Payment is split into:
///   - Interest portion  → cdp_fee_vault
///   - Principal portion → pool_vault
///
/// On full repayment with a collateral shortfall, the shortfall-equivalent SOL is
/// diverted to cdp_wsol_buyback_vault. Call claim_collateral afterward.
#[inline(never)]
fn route_spl<'info>(
    cdp_fee_vault: AccountInfo<'info>,
    pool_vault: AccountInfo<'info>,
    cdp_wsol_buyback_vault: AccountInfo<'info>,
    system_program: AccountInfo<'info>,
    fee_vault_bump: u8,
    principal_to_pool: u64,
    shortfall_sol_divert: u64,
) -> Result<()> {
    let fee_seeds = &[b"cdp_fee_vault".as_ref(), &[fee_vault_bump]];
    let fee_signer = &[&fee_seeds[..]];
    if principal_to_pool > 0 {
        system_program::transfer(
            CpiContext::new_with_signer(system_program.clone(), system_program::Transfer {
                from: cdp_fee_vault.clone(),
                to:   pool_vault.clone(),
            }, fee_signer),
            principal_to_pool,
        )?;
    }
    if shortfall_sol_divert > 0 {
        system_program::transfer(
            CpiContext::new_with_signer(system_program.clone(), system_program::Transfer {
                from: cdp_fee_vault.clone(),
                to:   cdp_wsol_buyback_vault.clone(),
            }, fee_signer),
            shortfall_sol_divert,
        )?;
    }
    Ok(())
}

pub fn handler(
    ctx: Context<RepayDebtSpl>,
    payment_amount: u64,
    route_plan_data: Vec<u8>,
    quoted_out_amount: u64,
    slippage_bps: u16,
) -> Result<()> {
    require!(payment_amount > 0, CdpError::ZeroAmount);

    // Swap SPL token → WSOL → SOL via Jupiter; unwrap WSOL to get actual SOL lamports.
    crate::jupiter::shared_accounts_route(
        &ctx.accounts.jupiter_program,
        &ctx.accounts.jupiter_program_authority,
        &ctx.accounts.borrower.to_account_info(),
        &ctx.accounts.borrower_payment_account.to_account_info(),
        &ctx.accounts.jupiter_source_token,
        &ctx.accounts.jupiter_destination_token,
        &ctx.accounts.cdp_wsol_vault.to_account_info(),
        &ctx.accounts.payment_mint.to_account_info(),
        &ctx.accounts.wsol_mint.to_account_info(),
        &ctx.accounts.jupiter_event_authority,
        &ctx.accounts.token_program.to_account_info(),
        &route_plan_data,
        payment_amount,
        quoted_out_amount,
        slippage_bps,
        &[],
    )?;

    ctx.accounts.cdp_wsol_vault.reload()?;
    let wsol_received = ctx.accounts.cdp_wsol_vault.amount;

    let cdp_config_bump = ctx.accounts.cdp_config.bump;
    let config_seeds = &[b"cdp_config".as_ref(), &[cdp_config_bump]];
    let config_signer = &[&config_seeds[..]];

    token::close_account(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account:     ctx.accounts.cdp_wsol_vault.to_account_info(),
                destination: ctx.accounts.cdp_fee_vault.to_account_info(),
                authority:   ctx.accounts.cdp_config.to_account_info(),
            },
            config_signer,
        ),
    )?;

    msg!("Jupiter swap: {} tokens → {} lamports SOL", payment_amount, wsol_received);
    let payment_sol_lamports = wsol_received;

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

    route_spl(
        ctx.accounts.cdp_fee_vault.to_account_info(),
        ctx.accounts.pool_vault.to_account_info(),
        ctx.accounts.cdp_wsol_buyback_vault.to_account_info(),
        ctx.accounts.system_program.to_account_info(),
        ctx.bumps.cdp_fee_vault,
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
pub struct RepayDebtSpl<'info> {
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
        seeds = [b"payment_config", payment_config.mint.as_ref()],
        bump = payment_config.bump,
        constraint = payment_config.active @ CdpError::PaymentConfigInactive,
    )]
    pub payment_config: Box<Account<'info, PaymentConfig>>,

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

    /// CHECK: PDA verified by seeds; holds native SOL; receives Jupiter swap output after WSOL unwrap.
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

    /// Pyth PriceUpdateV2 for collateral token.
    pub price_update: Box<Account<'info, PriceUpdateV2>>,

    /// Pyth PriceUpdateV2 for SOL/USD.
    pub sol_price_update: Box<Account<'info, PriceUpdateV2>>,

    /// SPL payment token mint (USDC, USDT, wBTC, wETH).
    pub payment_mint: Box<Account<'info, Mint>>,

    /// Borrower's SPL payment token account — Jupiter's swap source.
    #[account(mut)]
    pub borrower_payment_account: Box<Account<'info, TokenAccount>>,

    /// Native SOL (WSOL) mint.
    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Box<Account<'info, Mint>>,

    /// Protocol WSOL buffer: receives Jupiter's WSOL output, then closed to unwrap → native SOL.
    #[account(
        mut,
        seeds = [b"cdp_wsol_vault"],
        bump,
    )]
    pub cdp_wsol_vault: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [b"cdp_wsol_buyback_vault"],
        bump,
    )]
    pub cdp_wsol_buyback_vault: Box<Account<'info, TokenAccount>>,

    /// CHECK: Jupiter v6 program.
    #[account(address = crate::jupiter::PROGRAM_ID)]
    pub jupiter_program: AccountInfo<'info>,

    /// CHECK: Jupiter's shared authority PDA.
    pub jupiter_program_authority: AccountInfo<'info>,

    /// CHECK: Jupiter's event authority PDA.
    pub jupiter_event_authority: AccountInfo<'info>,

    /// CHECK: Jupiter's shared source token account.
    #[account(mut)]
    pub jupiter_source_token: AccountInfo<'info>,

    /// CHECK: Jupiter's shared destination token account.
    #[account(mut)]
    pub jupiter_destination_token: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,

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
}
