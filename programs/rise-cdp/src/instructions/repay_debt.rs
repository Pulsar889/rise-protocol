use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token::{self, Token, TokenAccount, Transfer as TokenTransfer, Mint};
use crate::state::{CdpPosition, CollateralConfig, PaymentConfig, CdpConfig};
use crate::errors::CdpError;
use rise_staking::state::GlobalPool;

/// Repay all or part of a CDP position's debt.
///
/// Accepts payment in native SOL or any SPL token configured in PaymentConfig
/// (USDC, USDT, BTC, ETH). The payment value is converted to a SOL equivalent
/// using Pyth prices, then split:
///   - Interest portion  → cdp_fee_vault  (swept later by collect_cdp_fees)
///   - Principal portion → pool_vault     (increases pool backing; riseSOL rate rises)
///
/// Interest is cleared before principal (standard lending convention).
/// On full repayment the position is closed and collateral returned to the borrower.
pub fn handler(ctx: Context<RepayDebt>, payment_amount: u64) -> Result<()> {
    require!(payment_amount > 0, CdpError::ZeroAmount);

    let payment_config = &ctx.accounts.payment_config;
    require!(payment_config.active, CdpError::PaymentConfigInactive);

    // ── Compute payment value in SOL lamports ───────────────────────────────
    let is_native_sol = payment_config.is_native_sol();

    let payment_sol_lamports: u64 = if is_native_sol {
        // Payment is already in lamports — no conversion needed.
        payment_amount
    } else {
        // SPL token: convert to SOL equivalent via Pyth prices.
        // TODO: Real Jupiter v6 CPI to swap payment token → SOL goes here.
        //       The lamports below represent the SOL value of the tokens;
        //       actual Jupiter integration will deliver real SOL to this program.
        let payment_price = get_mock_price(&ctx.accounts.pyth_price_feed)?;
        let sol_price = get_mock_price(&ctx.accounts.sol_price_feed)?;

        let token_decimals = ctx
            .accounts
            .payment_mint
            .as_ref()
            .expect("payment_mint required for SPL repayment")
            .decimals;
        let decimal_scale = 10u128.pow(token_decimals as u32);

        let payment_usd = (payment_amount as u128)
            .checked_mul(payment_price)
            .ok_or(CdpError::MathOverflow)?
            .checked_div(decimal_scale)
            .ok_or(CdpError::MathOverflow)?;

        let sol_lamports = payment_usd
            .checked_mul(1_000_000_000)
            .ok_or(CdpError::MathOverflow)?
            .checked_div(sol_price)
            .ok_or(CdpError::MathOverflow)?;

        u64::try_from(sol_lamports).map_err(|_| CdpError::MathOverflow)?
    };

    // ── Convert payment SOL → riseSOL units using current exchange rate ────────
    let exchange_rate = ctx.accounts.global_pool.exchange_rate; // SOL lamports per riseSOL * RATE_SCALE
    let rate_scale = GlobalPool::RATE_SCALE;

    // payment_rise_sol = payment_sol * RATE_SCALE / exchange_rate
    let payment_rise_sol_u128 = (payment_sol_lamports as u128)
        .checked_mul(rate_scale)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(exchange_rate)
        .ok_or(CdpError::MathOverflow)?;
    let payment_rise_sol = u64::try_from(payment_rise_sol_u128).map_err(|_| CdpError::MathOverflow)?;

    require!(payment_rise_sol > 0, CdpError::ZeroAmount);

    let position = &mut ctx.accounts.position;

    // ── Compute total outstanding debt and cap repayment ────────────────────
    let total_owed = position.total_rise_sol_owed().ok_or(CdpError::MathOverflow)?;
    require!(total_owed > 0, CdpError::ZeroAmount);

    let cleared_rise_sol = payment_rise_sol.min(total_owed);

    // ── Clear interest first, then principal ────────────────────────────────
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

    // ── Convert cleared riseSOL amounts back to SOL lamports for routing ───────
    // interest_sol = interest_cleared_rise_sol * exchange_rate / RATE_SCALE
    let interest_sol = (interest_cleared_rise_sol as u128)
        .checked_mul(exchange_rate)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(rate_scale)
        .ok_or(CdpError::MathOverflow)? as u64;

    // Total SOL being taken from borrower (use saturating to absorb rounding)
    let cleared_sol = (cleared_rise_sol as u128)
        .checked_mul(exchange_rate)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(rate_scale)
        .ok_or(CdpError::MathOverflow)? as u64;

    let principal_sol = cleared_sol.saturating_sub(interest_sol);

    // ── Route payment ───────────────────────────────────────────────────────
    if is_native_sol {
        // Interest → cdp_fee_vault
        if interest_sol > 0 {
            system_program::transfer(
                CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.borrower.to_account_info(),
                        to: ctx.accounts.cdp_fee_vault.to_account_info(),
                    },
                ),
                interest_sol,
            )?;
        }

        // Principal → pool_vault (appreciated by update_exchange_rate crank)
        if principal_sol > 0 {
            system_program::transfer(
                CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    system_program::Transfer {
                        from: ctx.accounts.borrower.to_account_info(),
                        to: ctx.accounts.pool_vault.to_account_info(),
                    },
                ),
                principal_sol,
            )?;
        }
    } else {
        // SPL token payment: transfer tokens into the stub payment vault.
        // TODO: Real Jupiter v6 CPI goes here to swap tokens → SOL, then
        //       route SOL splits to cdp_fee_vault and pool_vault above.
        //       Until Jupiter is integrated the token is held in payment_vault.
        let borrower_payment_account = ctx
            .accounts
            .borrower_payment_account
            .as_ref()
            .expect("borrower_payment_account required for SPL repayment");
        let payment_vault = ctx
            .accounts
            .payment_vault
            .as_ref()
            .expect("payment_vault required for SPL repayment");

        let cpi_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            TokenTransfer {
                from: borrower_payment_account.to_account_info(),
                to: payment_vault.to_account_info(),
                authority: ctx.accounts.borrower.to_account_info(),
            },
        );
        token::transfer(cpi_ctx, payment_amount)?;
        msg!("STUB: SPL tokens transferred to payment_vault. Jupiter CPI pending.");
    }

    // ── Decrement global CDP minted counter by principal cleared ────────────
    if principal_cleared_rise_sol > 0 {
        let cdp_config = &mut ctx.accounts.cdp_config;
        cdp_config.cdp_rise_sol_minted = cdp_config
            .cdp_rise_sol_minted
            .saturating_sub(principal_cleared_rise_sol as u128);
    }

    // ── Full repayment: return collateral and close position ─────────────────
    let is_fully_repaid =
        position.interest_accrued == 0 && position.rise_sol_debt_principal == 0;

    if is_fully_repaid {
        ctx.accounts.collateral_config.total_collateral_entitlements = ctx
            .accounts
            .collateral_config
            .total_collateral_entitlements
            .saturating_sub(position.collateral_amount_original);

        let collateral_config = &ctx.accounts.collateral_config;
        let config_mint_ref = collateral_config.mint.as_ref();
        let vault_bump = ctx.bumps.collateral_vault;
        let seeds = &[b"collateral_vault".as_ref(), config_mint_ref, &[vault_bump]];
        let signer = &[&seeds[..]];

        let owed = position.collateral_amount_original;
        let available = ctx.accounts.collateral_vault.amount.min(owed);
        let shortfall = owed.saturating_sub(available);

        // Transfer whatever collateral is in the vault
        if available > 0 {
            let cpi_ctx = CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                TokenTransfer {
                    from: ctx.accounts.collateral_vault.to_account_info(),
                    to: ctx.accounts.borrower_collateral_account.to_account_info(),
                    authority: ctx.accounts.collateral_vault.to_account_info(),
                },
                signer,
            );
            token::transfer(cpi_ctx, available)?;
        }

        // If collateral was previously seized for liquidity, buy it back using
        // the borrower's payment. The SOL value of the shortfall is diverted
        // from the principal payment (instead of going to pool_vault) and swapped
        // via Jupiter into the collateral token, then sent to the borrower.
        if shortfall > 0 {
            // TODO: Jupiter v6 CPI
            // 1. Calculate shortfall SOL value: shortfall_tokens * collateral_price / sol_price
            // 2. Divert shortfall_sol from principal (reduce pool_vault transfer above by that amount)
            // 3. Jupiter swap: shortfall_sol → collateral tokens
            // 4. Transfer collateral tokens to borrower_collateral_account
            msg!(
                "STUB: Collateral shortfall of {} tokens — Jupiter buyback pending",
                shortfall
            );
        }

        position.is_open = false;
        msg!(
            "Position fully repaid and closed. Collateral returned: {} (shortfall: {})",
            available,
            shortfall
        );
    }

    msg!("riseSOL interest cleared:   {}", interest_cleared_rise_sol);
    msg!("riseSOL principal cleared:  {}", principal_cleared_rise_sol);
    msg!("SOL to fee vault:        {}", interest_sol);
    msg!("SOL to pool backing:     {}", principal_sol);

    Ok(())
}

fn get_mock_price(price_feed: &AccountInfo) -> Result<u128> {
    // In production this uses the Pyth SDK to parse the price account.
    // For now we read a mock price stored in the account's lamports.
    let lamports = price_feed.lamports();
    require!(lamports > 0, CdpError::InvalidOraclePrice);
    Ok(lamports as u128)
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
        seeds = [b"payment_config", payment_config.mint.as_ref()],
        bump = payment_config.bump
    )]
    pub payment_config: Box<Account<'info, PaymentConfig>>,

    /// GlobalPool from the staking program — read-only for exchange rate.
    /// Must be the staking program's global_pool PDA.
    pub global_pool: Box<Account<'info, GlobalPool>>,

    /// Global CDP config — tracks total CDP riseSOL minted.
    #[account(
        mut,
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Box<Account<'info, CdpConfig>>,

    /// CHECK: CDP fee vault PDA — receives interest portion of repayment.
    #[account(
        mut,
        seeds = [b"cdp_fee_vault"],
        bump
    )]
    pub cdp_fee_vault: UncheckedAccount<'info>,

    /// CHECK: Staking pool SOL vault — receives principal portion to back riseSOL.
    /// Must be the staking program's pool_vault PDA. Validated by caller.
    #[account(mut)]
    pub pool_vault: UncheckedAccount<'info>,

    /// Protocol collateral vault — returns tokens to borrower on full repayment.
    #[account(
        mut,
        seeds = [b"collateral_vault", collateral_config.mint.as_ref()],
        bump,
        constraint = collateral_vault.mint == collateral_config.mint
    )]
    pub collateral_vault: Box<Account<'info, TokenAccount>>,

    /// Borrower's collateral account — receives collateral back on full repayment.
    #[account(
        mut,
        constraint = borrower_collateral_account.mint == collateral_config.mint,
        constraint = borrower_collateral_account.owner == borrower.key()
    )]
    pub borrower_collateral_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: Pyth price feed for the payment token. For native SOL pass the SOL feed.
    pub pyth_price_feed: AccountInfo<'info>,

    /// CHECK: Pyth price feed for SOL/USD (used in SPL token conversion path).
    pub sol_price_feed: AccountInfo<'info>,

    // ── SPL payment token accounts ──────────────────────────────────────────
    // Required for non-SOL payments; pass any pubkey (e.g. program ID) for native SOL.
    /// CHECK: Payment token mint. Ignored for native SOL payments.
    pub payment_mint: Option<Box<Account<'info, Mint>>>,

    /// Borrower's payment token account. Ignored for native SOL payments.
    #[account(mut)]
    pub borrower_payment_account: Option<Box<Account<'info, TokenAccount>>>,

    /// Stub payment vault for SPL tokens (pending Jupiter integration).
    #[account(mut)]
    pub payment_vault: Option<Box<Account<'info, TokenAccount>>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
