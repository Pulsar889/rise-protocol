use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer as TokenTransfer, Mint, SyncNative};
use crate::state::{CdpPosition, CollateralConfig, CdpConfig, PaymentConfig};
use crate::errors::CdpError;
use rise_staking::program::RiseStaking;
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;

/// Claim collateral from a fully-repaid CDP position.
///
/// Must be called after repay_debt or repay_debt_rise_sol has set position.is_open = false.
/// This instruction:
///   1. Decrements total_collateral_entitlements
///   2. Transfers available collateral from vault to borrower
///   3. If a shortfall exists (collateral was previously seized), executes the buyback:
///      - SOL/SPL repay path (pending_buyback_lamports > 0): uses diverted SOL already in
///        cdp_wsol_buyback_vault to Jupiter-swap WSOL → collateral for borrower
///      - riseSOL repay path (pending_buyback_lamports == 0): withdraws SOL from protocol
///        treasury and does the same swap
///   4. Closes the position account (rent returned to borrower)
///
/// The frontend should ensure borrower_collateral_account (ATA) exists before calling.

/// Extracted to keep handler stack frame small.
#[inline(never)]
fn compute_shortfall_sol(
    shortfall_tokens: u64,
    coll_price: u128,
    sol_price: u128,
    decimals: u8,
) -> Result<u64> {
    let dec_scale = 10u128.pow(decimals as u32);
    let sf_usd = (shortfall_tokens as u128)
        .checked_mul(coll_price).ok_or(CdpError::MathOverflow)?
        .checked_div(dec_scale).ok_or(CdpError::MathOverflow)?;
    let sf_sol = sf_usd
        .checked_mul(1_000_000_000u128).ok_or(CdpError::MathOverflow)?
        .checked_div(sol_price).ok_or(CdpError::MathOverflow)? as u64;
    Ok(sf_sol)
}

/// Extracted to keep handler stack frame small.
#[inline(never)]
fn run_jupiter_buyback<'a>(
    jupiter_program: &AccountInfo<'a>,
    jupiter_authority: &AccountInfo<'a>,
    cdp_config: &AccountInfo<'a>,
    buyback_vault: &AccountInfo<'a>,
    jup_source: &AccountInfo<'a>,
    jup_dest: &AccountInfo<'a>,
    borrower_collateral: &AccountInfo<'a>,
    wsol_mint: &AccountInfo<'a>,
    collateral_mint: &AccountInfo<'a>,
    jupiter_event: &AccountInfo<'a>,
    token_program: &AccountInfo<'a>,
    route_plan_data: &[u8],
    in_amount: u64,
    quoted_out_amount: u64,
    slippage_bps: u16,
    signer_seeds: &[&[&[u8]]],
) -> Result<()> {
    crate::jupiter::shared_accounts_route(
        jupiter_program,
        jupiter_authority,
        cdp_config,
        buyback_vault,
        jup_source,
        jup_dest,
        borrower_collateral,
        wsol_mint,
        collateral_mint,
        jupiter_event,
        token_program,
        route_plan_data,
        in_amount,
        quoted_out_amount,
        slippage_bps,
        signer_seeds,
    )
}

pub fn handler(
    ctx: Context<ClaimCollateral>,
    route_plan_data: Vec<u8>,
    quoted_out_amount: u64,
    slippage_bps: u16,
) -> Result<()> {
    let owed             = ctx.accounts.position.collateral_amount_original;
    let vault_balance    = ctx.accounts.collateral_vault.amount;
    let available        = vault_balance.min(owed);
    let shortfall_tokens = owed.saturating_sub(available);
    let pending_buyback  = ctx.accounts.position.pending_buyback_lamports;

    // ── Decrement total collateral entitlements ───────────────────────────────
    ctx.accounts.collateral_config.total_collateral_entitlements = ctx
        .accounts.collateral_config.total_collateral_entitlements
        .saturating_sub(owed);

    // ── Transfer available collateral to borrower ─────────────────────────────
    if available > 0 {
        let config_mint_ref = ctx.accounts.collateral_config.mint.as_ref();
        let vault_bump      = ctx.bumps.collateral_vault;
        let vault_seeds     = &[b"collateral_vault".as_ref(), config_mint_ref, &[vault_bump]];
        let vault_signer    = &[&vault_seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                TokenTransfer {
                    from:      ctx.accounts.collateral_vault.to_account_info(),
                    to:        ctx.accounts.borrower_collateral_account.to_account_info(),
                    authority: ctx.accounts.collateral_vault.to_account_info(),
                },
                vault_signer,
            ),
            available,
        )?;
    }

    // ── Shortfall buyback ─────────────────────────────────────────────────────
    if shortfall_tokens > 0 {
        let cdp_config_bump  = ctx.accounts.cdp_config.bump;
        let config_seeds     = &[b"cdp_config".as_ref(), &[cdp_config_bump]];
        let config_signer: &[&[&[u8]]] = &[&config_seeds[..]];

        let buyback_in_amount: u64;

        if pending_buyback > 0 {
            // SOL/SPL repay path: SOL was diverted into buyback vault during repay_debt.
            buyback_in_amount = pending_buyback;
        } else {
            // riseSOL repay path: no SOL was diverted. Use the protocol treasury.
            let coll_price = crate::pyth::get_pyth_price(
                &ctx.accounts.price_update,
                &ctx.accounts.collateral_config.pyth_price_feed.to_bytes(),
            )?;
            let sol_price = crate::pyth::get_pyth_price(
                &ctx.accounts.sol_price_update,
                &ctx.accounts.sol_payment_config.pyth_price_feed.to_bytes(),
            )?;
            let decimals  = ctx.accounts.collateral_mint.decimals;
            let shortfall_sol = compute_shortfall_sol(shortfall_tokens, coll_price, sol_price, decimals)?;

            if shortfall_sol > 0 {
                rise_staking::cpi::withdraw_treasury_for_cdp_buyback(
                    CpiContext::new_with_signer(
                        ctx.accounts.staking_program.to_account_info(),
                        rise_staking::cpi::accounts::WithdrawTreasuryForCdpBuyback {
                            cdp_config:             ctx.accounts.cdp_config.to_account_info(),
                            global_pool:            ctx.accounts.global_pool.to_account_info(),
                            treasury:               ctx.accounts.treasury.to_account_info(),
                            reserve_vault:          ctx.accounts.reserve_vault.to_account_info(),
                            cdp_wsol_buyback_vault: ctx.accounts.cdp_wsol_buyback_vault.to_account_info(),
                            system_program:         ctx.accounts.system_program.to_account_info(),
                        },
                        config_signer,
                    ),
                    shortfall_sol,
                )?;
            }

            buyback_in_amount = shortfall_sol;
        }

        if buyback_in_amount > 0 {
            // Wrap lamports in the buyback vault as WSOL.
            token::sync_native(CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                SyncNative {
                    account: ctx.accounts.cdp_wsol_buyback_vault.to_account_info(),
                },
            ))?;

            // Jupiter swap: cdp_wsol_buyback_vault (WSOL) → borrower_collateral_account
            run_jupiter_buyback(
                &ctx.accounts.jupiter_program,
                &ctx.accounts.jupiter_program_authority,
                &ctx.accounts.cdp_config.to_account_info(),
                &ctx.accounts.cdp_wsol_buyback_vault.to_account_info(),
                &ctx.accounts.jupiter_source_token,
                &ctx.accounts.jupiter_destination_token,
                &ctx.accounts.borrower_collateral_account.to_account_info(),
                &ctx.accounts.wsol_mint.to_account_info(),
                &ctx.accounts.collateral_mint.to_account_info(),
                &ctx.accounts.jupiter_event_authority,
                &ctx.accounts.token_program.to_account_info(),
                &route_plan_data,
                buyback_in_amount,
                quoted_out_amount,
                slippage_bps,
                config_signer,
            )?;

            msg!("Shortfall buyback complete: {} lamports WSOL → collateral tokens", buyback_in_amount);
        }
    }

    msg!(
        "Collateral claimed: {} tokens returned (shortfall: {}, buyback: {})",
        available,
        shortfall_tokens,
        pending_buyback,
    );

    // Position account is closed by the `close = borrower` constraint — rent reclaimed to borrower.
    Ok(())
}

#[derive(Accounts)]
pub struct ClaimCollateral<'info> {
    #[account(mut)]
    pub borrower: Signer<'info>,

    #[account(
        mut,
        seeds = [b"cdp_position", borrower.key().as_ref(), &[position.nonce]],
        bump = position.bump,
        constraint = position.owner == borrower.key(),
        constraint = !position.is_open @ CdpError::PositionStillOpen,
        close = borrower
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
        mut,
        seeds = [b"collateral_vault", collateral_config.mint.as_ref()],
        bump,
        constraint = collateral_vault.mint == collateral_config.mint
    )]
    pub collateral_vault: Box<Account<'info, TokenAccount>>,

    /// Borrower's token account for the collateral asset. Must be initialized (ATA) before calling.
    #[account(
        mut,
        constraint = borrower_collateral_account.mint == collateral_config.mint,
        constraint = borrower_collateral_account.owner == borrower.key()
    )]
    pub borrower_collateral_account: Box<Account<'info, TokenAccount>>,

    /// Collateral token mint — needed for decimal scaling in shortfall SOL computation.
    #[account(constraint = collateral_mint.key() == collateral_config.mint @ CdpError::CollateralNotAccepted)]
    pub collateral_mint: Box<Account<'info, Mint>>,

    /// Global CDP config — PDA signer for vault and staking CPIs.
    #[account(
        mut,
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Box<Account<'info, CdpConfig>>,

    /// Protocol WSOL buyback vault — holds diverted SOL (SOL path) or treasury SOL (riseSOL path).
    #[account(
        mut,
        seeds = [b"cdp_wsol_buyback_vault"],
        bump,
    )]
    pub cdp_wsol_buyback_vault: Box<Account<'info, TokenAccount>>,

    /// Native SOL (WSOL) mint.
    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Box<Account<'info, Mint>>,

    // ── Treasury path accounts (riseSOL repay path; passed but unused if pending_buyback > 0) ──

    pub staking_program: Program<'info, RiseStaking>,

    #[account(
        mut,
        seeds = [b"global_pool"],
        seeds::program = rise_staking::ID,
        bump = global_pool.bump
    )]
    pub global_pool: Box<Account<'info, rise_staking::state::GlobalPool>>,

    /// CHECK: Treasury account — validated by the staking program CPI.
    #[account(mut)]
    pub treasury: UncheckedAccount<'info>,

    /// CHECK: Reserve vault — validated by the staking program CPI.
    #[account(mut)]
    pub reserve_vault: UncheckedAccount<'info>,

    /// SOL payment config — provides the registered SOL/USD Pyth feed ID.
    #[account(
        seeds = [b"payment_config", anchor_lang::solana_program::system_program::ID.as_ref()],
        bump = sol_payment_config.bump,
    )]
    pub sol_payment_config: Box<Account<'info, PaymentConfig>>,

    /// Pyth PriceUpdateV2 for the collateral token.
    pub price_update: Account<'info, PriceUpdateV2>,

    /// Pyth PriceUpdateV2 for SOL/USD.
    pub sol_price_update: Account<'info, PriceUpdateV2>,

    // ── Jupiter accounts ────────────────────────────────────────────────────────

    /// CHECK: Jupiter v6 program.
    #[account(address = crate::jupiter::PROGRAM_ID)]
    pub jupiter_program: AccountInfo<'info>,

    /// CHECK: Jupiter's shared authority PDA.
    pub jupiter_program_authority: AccountInfo<'info>,

    /// CHECK: Jupiter's event authority PDA.
    pub jupiter_event_authority: AccountInfo<'info>,

    /// CHECK: Jupiter's shared source token account for the buyback route (WSOL side).
    #[account(mut)]
    pub jupiter_source_token: AccountInfo<'info>,

    /// CHECK: Jupiter's shared destination token account for the buyback route (collateral side).
    #[account(mut)]
    pub jupiter_destination_token: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
