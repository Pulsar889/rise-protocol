use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, Mint};
use crate::state::CollateralConfig;
use crate::errors::CdpError;
use rise_staking::state::GlobalPool;

/// Permissionless liquidity backstop. Anyone can call this when the staking pool's
/// liquid buffer cannot cover queued withdrawal tickets. The protocol seizes
/// collateral from the shared vault and holds it in a dedicated seizure vault
/// pending Jupiter conversion to SOL.
///
/// The borrower's position is NOT modified — their entitlement is still recorded
/// in CollateralConfig::total_collateral_entitlements. When they repay, the protocol
/// will source their collateral from other vaults or convert SOL if needed.
///
/// Flow:
///   1. Verify liquid_buffer_lamports < pending_withdrawals_lamports
///   2. Transfer `amount` tokens from collateral_vault → cdp_seizure_vault
///   3. TODO: Jupiter CPI converts cdp_seizure_vault tokens → SOL
///   4. TODO: Transfer SOL to pool_vault
///   5. TODO: CPI to rise_staking::receive_cdp_liquidity to register the SOL
///            as liquid buffer so withdrawal tickets can be paid out
pub fn handler(ctx: Context<RedeemCollateralForLiquidity>, amount: u64) -> Result<()> {
    require!(amount > 0, CdpError::ZeroAmount);

    // ── Condition check — only callable during a genuine liquidity shortfall ──
    let pool = &ctx.accounts.global_pool;
    require!(
        pool.liquid_buffer_lamports < pool.pending_withdrawals_lamports,
        CdpError::LiquidityRedemptionNotNeeded
    );

    // ── Verify vault has enough tokens to seize ───────────────────────────────
    require!(
        ctx.accounts.collateral_vault.amount >= amount,
        CdpError::InsufficientExcess
    );

    // ── Transfer collateral → seizure vault ──────────────────────────────────
    let config_mint_ref = ctx.accounts.collateral_config.mint.as_ref();
    let vault_bump = ctx.bumps.collateral_vault;
    let seeds = &[b"collateral_vault".as_ref(), config_mint_ref, &[vault_bump]];
    let signer = &[&seeds[..]];

    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.collateral_vault.to_account_info(),
                to: ctx.accounts.cdp_seizure_vault.to_account_info(),
                authority: ctx.accounts.collateral_vault.to_account_info(),
            },
            signer,
        ),
        amount,
    )?;

    msg!("Collateral seized for liquidity: {} tokens", amount);
    msg!(
        "Liquid buffer shortfall: {} lamports",
        pool.pending_withdrawals_lamports
            .saturating_sub(pool.liquid_buffer_lamports)
    );

    // TODO: Jupiter CPI — swap `amount` tokens from cdp_seizure_vault → SOL.
    //       After the swap, transfer the SOL output to pool_vault, then call:
    //       rise_staking::cpi::receive_cdp_liquidity(cpi_ctx, sol_received)
    //       to register the new SOL as liquid buffer.
    msg!("TODO: Jupiter swap cdp_seizure_vault → SOL → pool_vault + receive_cdp_liquidity CPI");

    Ok(())
}

#[derive(Accounts)]
pub struct RedeemCollateralForLiquidity<'info> {
    /// Permissionless — any caller can trigger when conditions are met.
    /// Pays rent for cdp_seizure_vault init if first seizure of this token type.
    #[account(mut)]
    pub caller: Signer<'info>,

    /// GlobalPool from staking — read to verify the liquidity shortfall condition.
    pub global_pool: Account<'info, GlobalPool>,

    #[account(
        seeds = [b"collateral_config", collateral_config.mint.as_ref()],
        bump = collateral_config.bump
    )]
    pub collateral_config: Account<'info, CollateralConfig>,

    pub collateral_mint: Account<'info, Mint>,

    /// Protocol collateral vault — tokens are seized from here.
    #[account(
        mut,
        seeds = [b"collateral_vault", collateral_config.mint.as_ref()],
        bump,
        constraint = collateral_vault.mint == collateral_config.mint
    )]
    pub collateral_vault: Account<'info, TokenAccount>,

    /// Holding account for seized tokens awaiting Jupiter conversion.
    /// Initialized on first seizure of this collateral type.
    #[account(
        init_if_needed,
        payer = caller,
        token::mint = collateral_mint,
        token::authority = collateral_vault,
        seeds = [b"cdp_seizure_vault", collateral_config.mint.as_ref()],
        bump
    )]
    pub cdp_seizure_vault: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
