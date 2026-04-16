use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, Mint, CloseAccount};
use crate::state::{CdpPosition, CollateralConfig, CdpConfig, BorrowRewards, BorrowRewardsConfig, PaymentConfig};
use crate::errors::CdpError;
use rise_staking::state::GlobalPool;
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;

/// Protocol-owned liquidation. Permissionless — any caller can trigger this on
/// an unhealthy position, but the program enforces validity. Proceeds go to the
/// protocol, not the caller, except for a trigger fee to incentivize bots.
///
/// Flow:
///   1. Verify health factor < 1.0 (program rejects invalid liquidations)
///   2. Caller receives liquidation_penalty_bps % of collateral (trigger fee)
///   3. Excess collateral above (debt + fee) returned to borrower
///   4. Jupiter CPI converts remaining debt-worth collateral → WSOL → SOL:
///        principal SOL → pool_vault  (maintains riseSOL backing)
///        interest SOL  → cdp_fee_vault (collected as fees via collect_cdp_fees)
///   5. Debt cancelled, position closed
pub fn handler(
    ctx: Context<Liquidate>,
    route_plan_data: Vec<u8>,
    quoted_out_amount: u64,
    slippage_bps: u16,
) -> Result<()> {
    let position = &mut ctx.accounts.position;
    let config = &ctx.accounts.collateral_config;

    require!(position.is_open, CdpError::PositionClosed);

    // ── Price feeds ──────────────────────────────────────────────────────────
    let collateral_usd_price = crate::pyth::get_pyth_price(&ctx.accounts.price_update, &ctx.accounts.collateral_config.pyth_price_feed.to_bytes())?;
    let sol_usd_price = crate::pyth::get_pyth_price(&ctx.accounts.sol_price_update, &ctx.accounts.sol_payment_config.pyth_price_feed.to_bytes())?;

    let token_decimals = ctx.accounts.collateral_mint.decimals;
    let decimal_scale = 10u128.pow(token_decimals as u32);

    // ── Collateral and debt USD values ───────────────────────────────────────
    let collateral_usd = (position.collateral_amount_original as u128)
        .checked_mul(collateral_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(decimal_scale)
        .ok_or(CdpError::MathOverflow)?;

    let total_owed = position.total_rise_sol_owed().ok_or(CdpError::MathOverflow)?;
    let debt_usd = (total_owed as u128)
        .checked_mul(sol_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(1_000_000_000)
        .ok_or(CdpError::MathOverflow)?;

    // ── Health check — program rejects if position is still healthy ──────────
    let health_factor = CdpPosition::compute_health_factor(
        collateral_usd,
        debt_usd,
        config.liquidation_threshold_bps,
    ).ok_or(CdpError::MathOverflow)?;

    require!(
        health_factor < CollateralConfig::RATE_SCALE,
        CdpError::PositionHealthy
    );

    // ── Excess collateral → borrower (above debt value) ─────────────────────
    // Trigger fee is paid post-swap in SOL; excess is still returned as tokens.
    let excess_usd = collateral_usd.saturating_sub(debt_usd);

    let excess_tokens = excess_usd
        .checked_mul(decimal_scale)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(collateral_usd_price)
        .ok_or(CdpError::MathOverflow)? as u64;

    // ── Expected SOL splits (proportioning of actual Jupiter output) ──────────
    let exchange_rate = ctx.accounts.global_pool.exchange_rate;
    let rate_scale = GlobalPool::RATE_SCALE;

    let principal_sol_target = (position.rise_sol_debt_principal as u128)
        .checked_mul(exchange_rate)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(rate_scale)
        .ok_or(CdpError::MathOverflow)? as u64;

    let interest_sol_target = (position.interest_accrued as u128)
        .checked_mul(exchange_rate)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(rate_scale)
        .ok_or(CdpError::MathOverflow)? as u64;

    // ── Mark position closed before any CPI to prevent reentrancy ───────────
    // Setting this before Jupiter executes means any reentrant call to
    // liquidate will fail the is_open check. Transaction atomicity ensures
    // this reverts if anything below fails.
    position.is_open = false;

    // ── Vault signer seeds ───────────────────────────────────────────────────
    let config_mint_ref = config.mint.as_ref();
    let vault_bump = ctx.bumps.collateral_vault;
    let vault_seeds = &[b"collateral_vault".as_ref(), config_mint_ref, &[vault_bump]];
    let vault_signer = &[&vault_seeds[..]];

    // ── Excess → borrower ────────────────────────────────────────────────────
    if excess_tokens > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from:      ctx.accounts.collateral_vault.to_account_info(),
                    to:        ctx.accounts.borrower_collateral_account.to_account_info(),
                    authority: ctx.accounts.collateral_vault.to_account_info(),
                },
                vault_signer,
            ),
            excess_tokens,
        )?;
    }

    // ── Collateral to swap: everything except what was returned to borrower ───
    // Trigger fee (liquidation_penalty_bps %) will be paid post-swap in SOL.
    let debt_worth_tokens = position.collateral_amount_original
        .saturating_sub(excess_tokens);

    if debt_worth_tokens > 0 {
        // Jupiter v6 CPI: collateral_vault → cdp_wsol_vault (WSOL)
        // collateral_vault is both the source and the user_transfer_authority.
        crate::jupiter::shared_accounts_route(
            &ctx.accounts.jupiter_program,
            &ctx.accounts.jupiter_program_authority,
            &ctx.accounts.collateral_vault.to_account_info(),
            &ctx.accounts.collateral_vault.to_account_info(),
            &ctx.accounts.jupiter_source_token,
            &ctx.accounts.jupiter_destination_token,
            &ctx.accounts.cdp_wsol_vault.to_account_info(),
            &ctx.accounts.collateral_mint.to_account_info(),
            &ctx.accounts.wsol_mint.to_account_info(),
            &ctx.accounts.jupiter_event_authority,
            &ctx.accounts.token_program.to_account_info(),
            &route_plan_data,
            debt_worth_tokens,
            quoted_out_amount,
            slippage_bps,
            vault_signer,
        )?;

        // Record actual WSOL received
        ctx.accounts.cdp_wsol_vault.reload()?;
        let actual_sol = ctx.accounts.cdp_wsol_vault.amount;

        // Trigger fee = liquidation_penalty_bps % of actual SOL recovered
        let trigger_fee_sol = (actual_sol as u128)
            .checked_mul(config.liquidation_penalty_bps as u128)
            .ok_or(CdpError::MathOverflow)?
            .checked_div(10_000)
            .ok_or(CdpError::MathOverflow)? as u64;

        let sol_after_fee = actual_sol.saturating_sub(trigger_fee_sol);

        // Proportionally split remaining SOL between interest and principal
        let total_target = principal_sol_target.saturating_add(interest_sol_target);
        let actual_interest_sol = if total_target > 0 && sol_after_fee > 0 {
            (sol_after_fee as u128)
                .checked_mul(interest_sol_target as u128)
                .unwrap_or(0)
                .checked_div(total_target as u128)
                .unwrap_or(0) as u64
        } else {
            0
        };
        let actual_principal_sol = sol_after_fee.saturating_sub(actual_interest_sol);

        // Unwrap WSOL → native SOL: close cdp_wsol_vault → cdp_fee_vault
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

        let fee_vault_bump = ctx.bumps.cdp_fee_vault;
        let fee_seeds = &[b"cdp_fee_vault".as_ref(), &[fee_vault_bump]];
        let fee_signer = &[&fee_seeds[..]];

        // Trigger fee → caller (in SOL)
        if trigger_fee_sol > 0 {
            system_program::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.cdp_fee_vault.to_account_info(),
                        to:   ctx.accounts.caller.to_account_info(),
                    },
                    fee_signer,
                ),
                trigger_fee_sol,
            )?;
        }

        // Route principal SOL from cdp_fee_vault → pool_vault
        if actual_principal_sol > 0 {
            system_program::transfer(
                CpiContext::new_with_signer(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.cdp_fee_vault.to_account_info(),
                        to:   ctx.accounts.pool_vault.to_account_info(),
                    },
                    fee_signer,
                ),
                actual_principal_sol,
            )?;
        }
        // actual_interest_sol (+ rent) remains in cdp_fee_vault

        msg!("Jupiter swap: {} tokens → {} lamports SOL", debt_worth_tokens, actual_sol);
        msg!("  trigger fee SOL → caller:      {}", trigger_fee_sol);
        msg!("  principal SOL → pool_vault:    {}", actual_principal_sol);
        msg!("  interest  SOL → cdp_fee_vault: {}", actual_interest_sol);
    }

    // ── Decrement entitlement counter ────────────────────────────────────────
    ctx.accounts.collateral_config.total_collateral_entitlements = ctx
        .accounts
        .collateral_config
        .total_collateral_entitlements
        .saturating_sub(position.collateral_amount_original);

    // ── Settle borrow rewards and update global debt tracker ─────────────────
    {
        let reward_per_token = ctx.accounts.borrow_rewards_config.reward_per_token;
        let current_debt = position.rise_sol_debt_principal;
        ctx.accounts.borrow_rewards.settle(reward_per_token, current_debt)?;
        // Sync debt to zero so future settle() calls on the closed position produce 0.
        ctx.accounts.borrow_rewards.sync_debt(reward_per_token, 0)?;

        ctx.accounts.borrow_rewards_config.total_cdp_debt = ctx
            .accounts.borrow_rewards_config.total_cdp_debt
            .saturating_sub(current_debt);
    }

    // ── Decrement global CDP minted counter ──────────────────────────────────
    let cdp_config = &mut ctx.accounts.cdp_config;
    cdp_config.cdp_rise_sol_minted = cdp_config
        .cdp_rise_sol_minted
        .saturating_sub(position.rise_sol_debt_principal as u128);

    // ── Cancel debt ──────────────────────────────────────────────────────────
    position.rise_sol_debt_principal = 0;
    position.interest_accrued = 0;

    msg!("Position liquidated — health factor was: {}", health_factor);
    msg!("Excess returned to borrower: {} tokens", excess_tokens);

    Ok(())
}


#[derive(Accounts)]
pub struct Liquidate<'info> {
    /// Permissionless — any caller can trigger a valid liquidation.
    /// Receives the trigger fee; pays rent for cdp_wsol_vault if first use.
    #[account(mut)]
    pub caller: Signer<'info>,

    #[account(
        mut,
        seeds = [b"cdp_position", position.owner.as_ref(), &[position.nonce]],
        bump = position.bump,
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

    #[account(constraint = collateral_mint.key() == collateral_config.mint @ CdpError::CollateralNotAccepted)]
    pub collateral_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [b"collateral_vault", collateral_config.mint.as_ref()],
        bump,
        constraint = collateral_vault.mint == collateral_config.mint
    )]
    pub collateral_vault: Box<Account<'info, TokenAccount>>,

    /// Borrower's collateral account — receives excess collateral if any.
    #[account(
        mut,
        constraint = borrower_collateral_account.mint == collateral_config.mint,
        constraint = borrower_collateral_account.owner == position.owner @ CdpError::Unauthorized
    )]
    pub borrower_collateral_account: Box<Account<'info, TokenAccount>>,

    /// GlobalPool from staking — read for exchange rate.
    #[account(
        seeds = [b"global_pool"],
        seeds::program = rise_staking::ID,
        bump = global_pool.bump
    )]
    pub global_pool: Box<Account<'info, GlobalPool>>,

    /// Global CDP config — authority for cdp_wsol_vault; tracks total minted.
    #[account(
        mut,
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Box<Account<'info, CdpConfig>>,

    /// CDP fee vault — receives interest portion of Jupiter swap output.
    /// CHECK: PDA verified by seeds; holds native SOL.
    #[account(
        mut,
        seeds = [b"cdp_fee_vault"],
        bump
    )]
    pub cdp_fee_vault: UncheckedAccount<'info>,

    /// CHECK: Staking pool SOL vault — receives principal portion of Jupiter swap output.
    #[account(
        mut,
        seeds = [b"pool_vault"],
        seeds::program = rise_staking::ID,
        bump
    )]
    pub pool_vault: UncheckedAccount<'info>,

    /// Native SOL (WSOL) mint — Jupiter outputs WSOL which is then unwrapped.
    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Box<Account<'info, Mint>>,

    /// Protocol WSOL buffer: receives Jupiter's WSOL output, then closed to unwrap.
    #[account(
        init_if_needed,
        payer = caller,
        token::mint = wsol_mint,
        token::authority = cdp_config,
        seeds = [b"cdp_wsol_vault"],
        bump,
    )]
    pub cdp_wsol_vault: Box<Account<'info, TokenAccount>>,

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
    pub system_program: Program<'info, System>,

    // ── Jupiter v6 accounts ──────────────────────────────────────────────────

    /// CHECK: Jupiter v6 program.
    #[account(address = crate::jupiter::PROGRAM_ID)]
    pub jupiter_program: AccountInfo<'info>,

    /// CHECK: Jupiter's shared authority PDA.
    pub jupiter_program_authority: AccountInfo<'info>,

    /// CHECK: Jupiter's event authority PDA.
    pub jupiter_event_authority: AccountInfo<'info>,

    /// CHECK: Jupiter's shared source token account for this route (from Jupiter quote API).
    #[account(mut)]
    pub jupiter_source_token: AccountInfo<'info>,

    /// CHECK: Jupiter's shared destination token account for this route (from Jupiter quote API).
    #[account(mut)]
    pub jupiter_destination_token: AccountInfo<'info>,

    // ── Borrow rewards ───────────────────────────────────────────────────────

    #[account(
        mut,
        seeds = [b"borrow_rewards_config"],
        bump = borrow_rewards_config.bump
    )]
    pub borrow_rewards_config: Box<Account<'info, BorrowRewardsConfig>>,

    /// Borrow rewards — settled here so pending RISE is preserved for the borrower to claim
    /// via claim_borrow_rewards after liquidation. Account remains open intentionally.
    #[account(
        mut,
        seeds = [b"borrow_rewards", position.key().as_ref()],
        bump = borrow_rewards.bump,
        constraint = borrow_rewards.position == position.key()
    )]
    pub borrow_rewards: Box<Account<'info, BorrowRewards>>,
}
