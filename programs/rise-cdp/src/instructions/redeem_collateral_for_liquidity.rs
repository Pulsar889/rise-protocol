use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, Mint, CloseAccount};
use crate::state::{CollateralConfig, CdpConfig, PaymentConfig};
use crate::errors::CdpError;
use rise_staking::state::GlobalPool;

/// Permissionless liquidity backstop. Anyone can call this when the staking pool's
/// liquid buffer cannot cover queued withdrawal tickets. The protocol seizes
/// collateral from the shared vault, converts it to SOL via Jupiter, deposits the SOL
/// into pool_vault, and registers the new liquid buffer so withdrawal tickets can be paid.
///
/// The borrower's position is NOT modified — their entitlement is still recorded
/// in CollateralConfig::total_collateral_entitlements. When they repay, the protocol
/// will source their collateral from other vaults or convert SOL if needed.
///
/// Flow:
///   1. Verify liquid_buffer_lamports < pending_withdrawals_lamports
///   2. Transfer `amount` tokens: collateral_vault → cdp_seizure_vault
///   3. Jupiter CPI: cdp_seizure_vault → cdp_wsol_vault (WSOL)
///   4. Close cdp_wsol_vault → pool_vault (unwraps WSOL to native SOL)
///   5. CPI rise_staking::receive_cdp_liquidity to register new liquid buffer
pub fn handler(
    ctx: Context<RedeemCollateralForLiquidity>,
    amount: u64,
    route_plan_data: Vec<u8>,
    quoted_out_amount: u64,
    slippage_bps: u16,
) -> Result<()> {
    require!(amount > 0, CdpError::ZeroAmount);

    // NOTE (I-2): There is intentionally no `collateral_config.active` check here.
    // This function is a liquidity backstop — it must remain callable even when a
    // collateral type is being deprecated, because withdrawal tickets already queued
    // from that collateral still need to be honoured. Blocking backstop calls on an
    // inactive collateral would leave stakers unable to redeem.

    // ── Condition check — only callable during a genuine liquidity shortfall ──
    let pool = &ctx.accounts.global_pool;
    require!(
        pool.liquid_buffer_lamports < pool.pending_withdrawals_lamports,
        CdpError::LiquidityRedemptionNotNeeded
    );

    // ── Cap amount to what is actually needed to cover the shortfall ──────────
    // Prevents callers from seizing more collateral than necessary, which would
    // haircut all borrowers' entitlements beyond what the situation requires.
    let collateral_usd_price = crate::pyth::get_pyth_price(&ctx.accounts.pyth_price_feed)?;
    let sol_usd_price = crate::pyth::get_pyth_price(&ctx.accounts.sol_price_feed)?;

    let shortfall_lamports = pool.pending_withdrawals_lamports
        .saturating_sub(pool.liquid_buffer_lamports);

    let token_decimals = ctx.accounts.collateral_mint.decimals;
    let decimal_scale = 10u128.pow(token_decimals as u32);

    // shortfall in micro-USD, then convert to collateral token units
    let shortfall_usd = (shortfall_lamports as u128)
        .checked_mul(sol_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(1_000_000_000)
        .ok_or(CdpError::MathOverflow)?;

    let max_tokens_needed = shortfall_usd
        .checked_mul(decimal_scale)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(collateral_usd_price)
        .ok_or(CdpError::MathOverflow)? as u64;

    let amount = amount.min(max_tokens_needed);

    require!(amount > 0, CdpError::ZeroAmount);

    require!(
        ctx.accounts.collateral_vault.amount >= amount,
        CdpError::InsufficientExcess
    );

    // ── Vault signer seeds ───────────────────────────────────────────────────
    let config_mint_ref = ctx.accounts.collateral_config.mint.as_ref();
    let vault_bump = ctx.bumps.collateral_vault;
    let vault_seeds = &[b"collateral_vault".as_ref(), config_mint_ref, &[vault_bump]];
    let vault_signer = &[&vault_seeds[..]];

    // ── Transfer collateral → seizure vault ──────────────────────────────────
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from:      ctx.accounts.collateral_vault.to_account_info(),
                to:        ctx.accounts.cdp_seizure_vault.to_account_info(),
                authority: ctx.accounts.collateral_vault.to_account_info(),
            },
            vault_signer,
        ),
        amount,
    )?;

    msg!("Collateral seized for liquidity: {} tokens", amount);
    msg!(
        "Liquid buffer shortfall: {} lamports",
        pool.pending_withdrawals_lamports
            .saturating_sub(pool.liquid_buffer_lamports)
    );

    // ── Jupiter v6 CPI: cdp_seizure_vault → cdp_wsol_vault (WSOL) ────────────
    // cdp_seizure_vault is owned by collateral_vault — sign with vault seeds.
    crate::jupiter::shared_accounts_route(
        &ctx.accounts.jupiter_program,
        &ctx.accounts.jupiter_program_authority,
        &ctx.accounts.collateral_vault.to_account_info(),    // user_transfer_authority
        &ctx.accounts.cdp_seizure_vault.to_account_info(),  // source
        &ctx.accounts.jupiter_source_token,
        &ctx.accounts.jupiter_destination_token,
        &ctx.accounts.cdp_wsol_vault.to_account_info(),     // destination (WSOL)
        &ctx.accounts.collateral_mint.to_account_info(),
        &ctx.accounts.wsol_mint.to_account_info(),
        &ctx.accounts.jupiter_event_authority,
        &ctx.accounts.token_program.to_account_info(),
        &route_plan_data,
        amount,
        quoted_out_amount,
        slippage_bps,
        vault_signer,
    )?;

    // Record actual WSOL received before closing
    ctx.accounts.cdp_wsol_vault.reload()?;
    let sol_received = ctx.accounts.cdp_wsol_vault.amount;

    // ── Unwrap WSOL → native SOL directly into pool_vault ────────────────────
    let cdp_config_bump = ctx.accounts.cdp_config.bump;
    let config_seeds = &[b"cdp_config".as_ref(), &[cdp_config_bump]];
    let config_signer = &[&config_seeds[..]];

    token::close_account(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account:     ctx.accounts.cdp_wsol_vault.to_account_info(),
                destination: ctx.accounts.pool_vault.to_account_info(),
                authority:   ctx.accounts.cdp_config.to_account_info(),
            },
            config_signer,
        ),
    )?;

    // ── Register new liquid buffer with the staking program ──────────────────
    rise_staking::cpi::receive_cdp_liquidity(
        CpiContext::new(
            ctx.accounts.staking_program.to_account_info(),
            rise_staking::cpi::accounts::ReceiveCdpLiquidity {
                caller:      ctx.accounts.caller.to_account_info(),
                global_pool: ctx.accounts.global_pool.to_account_info(),
                pool_vault:  ctx.accounts.pool_vault.to_account_info(),
            },
        ),
        sol_received,
    )?;

    msg!("Jupiter swap: {} tokens → {} lamports SOL → pool_vault", amount, sol_received);

    Ok(())
}

#[derive(Accounts)]
pub struct RedeemCollateralForLiquidity<'info> {
    /// Permissionless — any caller can trigger when conditions are met.
    /// Pays rent for cdp_wsol_vault / cdp_seizure_vault init if first use.
    #[account(mut)]
    pub caller: Signer<'info>,

    /// GlobalPool from staking — read to verify the liquidity shortfall condition.
    #[account(
        mut,
        seeds = [b"global_pool"],
        seeds::program = rise_staking::ID,
        bump = global_pool.bump
    )]
    pub global_pool: Account<'info, GlobalPool>,

    #[account(
        seeds = [b"collateral_config", collateral_config.mint.as_ref()],
        bump = collateral_config.bump
    )]
    pub collateral_config: Account<'info, CollateralConfig>,

    #[account(constraint = collateral_mint.key() == collateral_config.mint @ CdpError::CollateralNotAccepted)]
    pub collateral_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [b"collateral_vault", collateral_config.mint.as_ref()],
        bump,
        constraint = collateral_vault.mint == collateral_config.mint
    )]
    pub collateral_vault: Account<'info, TokenAccount>,

    /// Intermediate holding account for seized tokens. Authority = collateral_vault
    /// so it can sign as user_transfer_authority for Jupiter.
    #[account(
        init_if_needed,
        payer = caller,
        token::mint = collateral_mint,
        token::authority = collateral_vault,
        seeds = [b"cdp_seizure_vault", collateral_config.mint.as_ref()],
        bump
    )]
    pub cdp_seizure_vault: Account<'info, TokenAccount>,

    /// Global CDP config — authority for cdp_wsol_vault.
    #[account(
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Account<'info, CdpConfig>,

    /// Native SOL (WSOL) mint — Jupiter outputs WSOL which is then unwrapped.
    #[account(address = anchor_spl::token::spl_token::native_mint::ID)]
    pub wsol_mint: Account<'info, Mint>,

    /// Protocol WSOL buffer: receives Jupiter's WSOL output, then closed → pool_vault.
    #[account(
        init_if_needed,
        payer = caller,
        token::mint = wsol_mint,
        token::authority = cdp_config,
        seeds = [b"cdp_wsol_vault"],
        bump,
    )]
    pub cdp_wsol_vault: Account<'info, TokenAccount>,

    /// CHECK: Staking pool SOL vault — receives unwrapped SOL from Jupiter output.
    #[account(
        mut,
        seeds = [b"pool_vault"],
        seeds::program = rise_staking::ID,
        bump
    )]
    pub pool_vault: UncheckedAccount<'info>,

    /// SOL payment config — provides the registered SOL/USD price feed pubkey for validation.
    #[account(
        seeds = [b"payment_config", anchor_lang::solana_program::system_program::ID.as_ref()],
        bump = sol_payment_config.bump,
    )]
    pub sol_payment_config: Box<Account<'info, PaymentConfig>>,

    /// CHECK: Pyth price feed for the collateral token — must match collateral_config.pyth_price_feed.
    #[account(constraint = pyth_price_feed.key() == collateral_config.pyth_price_feed @ CdpError::WrongPriceFeed)]
    pub pyth_price_feed: AccountInfo<'info>,

    /// CHECK: Pyth price feed for SOL/USD — must match sol_payment_config.pyth_price_feed.
    #[account(constraint = sol_price_feed.key() == sol_payment_config.pyth_price_feed @ CdpError::WrongPriceFeed)]
    pub sol_price_feed: AccountInfo<'info>,

    pub staking_program: Program<'info, rise_staking::program::RiseStaking>,
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

    /// CHECK: Jupiter's shared source token account for this route.
    #[account(mut)]
    pub jupiter_source_token: AccountInfo<'info>,

    /// CHECK: Jupiter's shared destination token account for this route.
    #[account(mut)]
    pub jupiter_destination_token: AccountInfo<'info>,
}
