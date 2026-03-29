use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, Mint};
use crate::state::{CdpPosition, CollateralConfig, CdpConfig, BorrowRewards, BorrowRewardsConfig};
use crate::errors::CdpError;
use rise_staking::state::GlobalPool;
use rise_staking::program::RiseStaking;

pub fn handler(
    ctx: Context<OpenPosition>,
    collateral_amount: u64,
    rise_sol_to_mint: u64,
    nonce: u8,
) -> Result<()> {
    // Copy config fields needed later so we don't hold a borrow past the mutable update.
    let config_active = ctx.accounts.collateral_config.active;
    let config_max_ltv_bps = ctx.accounts.collateral_config.max_ltv_bps;
    let config_liquidation_threshold_bps = ctx.accounts.collateral_config.liquidation_threshold_bps;
    let config_mint = ctx.accounts.collateral_config.mint;

    require!(config_active, CdpError::CollateralNotAccepted);
    require!(collateral_amount > 0, CdpError::ZeroAmount);
    require!(rise_sol_to_mint > 0, CdpError::ZeroAmount);

    // --- Price validation ---
    // Get collateral USD price from Pyth
    let collateral_usd_price = get_pyth_price(&ctx.accounts.pyth_price_feed)?;

    // Get SOL USD price from Pyth
    let sol_usd_price = get_pyth_price(&ctx.accounts.sol_price_feed)?;

    // Calculate collateral USD value
    // collateral_usd = amount * price / 10^token_decimals
    let token_decimals = ctx.accounts.collateral_mint.decimals;
    let decimal_scale = 10u128.pow(token_decimals as u32);

    let collateral_usd_value = (collateral_amount as u128)
        .checked_mul(collateral_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(decimal_scale)
        .ok_or(CdpError::MathOverflow)?;

    // Calculate max riseSOL mintable at chosen LTV
    // max_borrow_usd = collateral_usd * max_ltv_bps / 10000
    // max_borrow_sol = max_borrow_usd / sol_usd_price
    // max_rise_sol = max_borrow_sol (at exchange rate 1.0 for simplicity in v1)
    let max_borrow_usd = collateral_usd_value
        .checked_mul(config_max_ltv_bps as u128)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(10_000)
        .ok_or(CdpError::MathOverflow)?;

    let max_borrow_lamports = max_borrow_usd
        .checked_mul(1_000_000_000) // lamports per SOL
        .ok_or(CdpError::MathOverflow)?
        .checked_div(sol_usd_price)
        .ok_or(CdpError::MathOverflow)?;

    require!(
        rise_sol_to_mint as u128 <= max_borrow_lamports,
        CdpError::ExceedsMaxLtv
    );

    // --- Debt ceiling check ---
    {
        let cdp_config = &mut ctx.accounts.cdp_config;
        let staking_supply = ctx.accounts.global_pool.staking_rise_sol_supply;
        let ceiling = staking_supply
            .checked_mul(cdp_config.debt_ceiling_multiplier_bps as u128)
            .ok_or(CdpError::MathOverflow)?
            .checked_div(10_000)
            .ok_or(CdpError::MathOverflow)?;

        let new_minted = cdp_config.cdp_rise_sol_minted
            .checked_add(rise_sol_to_mint as u128)
            .ok_or(CdpError::MathOverflow)?;

        require!(new_minted <= ceiling, CdpError::DebtCeilingExceeded);

        let single_loan_cap = ceiling
            .checked_mul(CdpConfig::MAX_SINGLE_LOAN_BPS)
            .ok_or(CdpError::MathOverflow)?
            .checked_div(10_000)
            .ok_or(CdpError::MathOverflow)?;

        require!(
            rise_sol_to_mint as u128 <= single_loan_cap,
            CdpError::ExceedsSingleLoanCap
        );

        cdp_config.cdp_rise_sol_minted = new_minted;
    }

    // --- Update entitlement counter (config borrow must be released first) ---
    let new_entitlements = ctx.accounts.collateral_config.total_collateral_entitlements
        .checked_add(collateral_amount)
        .ok_or(CdpError::MathOverflow)?;
    ctx.accounts.collateral_config.total_collateral_entitlements = new_entitlements;

    // --- Transfer collateral from borrower to protocol vault ---
    let cpi_ctx = CpiContext::new(
        ctx.accounts.token_program.to_account_info(),
        Transfer {
            from: ctx.accounts.borrower_collateral_account.to_account_info(),
            to: ctx.accounts.collateral_vault.to_account_info(),
            authority: ctx.accounts.borrower.to_account_info(),
        },
    );
    token::transfer(cpi_ctx, collateral_amount)?;

    // --- Initialize position state ---
    let position = &mut ctx.accounts.position;
    let current_slot = Clock::get()?.slot;

    position.owner = ctx.accounts.borrower.key();
    position.collateral_mint = config_mint;
    position.collateral_amount_original = collateral_amount;
    position.collateral_usd_value = collateral_usd_value;
    position.rise_sol_debt_principal = rise_sol_to_mint;
    position.interest_accrued = 0;
    position.last_accrual_slot = current_slot;
    position.opened_at_slot = current_slot;
    position.nonce = nonce;
    position.is_open = true;
    position.excess_withdrawal_queued = 0;
    position.excess_withdrawal_available_slot = 0;
    position.bump = ctx.bumps.position;

    // Calculate initial health factor
    let debt_usd = (rise_sol_to_mint as u128)
        .checked_mul(sol_usd_price)
        .ok_or(CdpError::MathOverflow)?
        .checked_div(1_000_000_000)
        .ok_or(CdpError::MathOverflow)?;

    position.health_factor = CdpPosition::compute_health_factor(
        collateral_usd_value,
        debt_usd,
        config_liquidation_threshold_bps,
    ).ok_or(CdpError::MathOverflow)?;

    // ── Mint riseSOL to borrower via CPI to staking program ─────────────────
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
        rise_sol_to_mint,
    )?;

    // ── Initialize BorrowRewards account for this position ───────────────────
    let borrow_rewards = &mut ctx.accounts.borrow_rewards;
    let reward_per_token = ctx.accounts.borrow_rewards_config.reward_per_token;

    borrow_rewards.owner = ctx.accounts.borrower.key();
    borrow_rewards.position = position.key();
    borrow_rewards.pending_rewards = 0;
    borrow_rewards.total_claimed = 0;
    borrow_rewards.last_checkpoint_slot = current_slot;
    borrow_rewards.bump = ctx.bumps.borrow_rewards;
    // reward_debt = initial_debt * reward_per_token / REWARD_SCALE
    borrow_rewards.sync_debt(reward_per_token, rise_sol_to_mint)?;

    // ── Update global debt tracker ───────────────────────────────────────────
    let brc = &mut ctx.accounts.borrow_rewards_config;
    brc.total_cdp_debt = brc.total_cdp_debt
        .checked_add(rise_sol_to_mint)
        .ok_or(CdpError::MathOverflow)?;

    msg!("Position opened");
    msg!("Collateral: {} tokens", collateral_amount);
    msg!("Collateral USD value: {}", collateral_usd_value);
    msg!("riseSOL minted: {}", rise_sol_to_mint);
    msg!("Health factor: {}", position.health_factor);

    Ok(())
}

/// Read and validate a Pyth price feed.
/// Returns price scaled by CollateralConfig::PRICE_SCALE (6 decimals).
fn get_pyth_price(price_feed: &AccountInfo) -> Result<u128> {
    // In production this uses the Pyth SDK to parse the price account.
    // For now we read a mock price stored in the account's lamports
    // as a placeholder. This will be replaced with real Pyth integration.
    let lamports = price_feed.lamports();
    require!(lamports > 0, CdpError::InvalidOraclePrice);
    Ok(lamports as u128)
}

#[derive(Accounts)]
#[instruction(collateral_amount: u64, rise_sol_to_mint: u64, nonce: u8)]
pub struct OpenPosition<'info> {
    #[account(mut)]
    pub borrower: Signer<'info>,

    /// Global CDP config — tracks total CDP riseSOL minted and debt ceiling.
    #[account(
        mut,
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Box<Account<'info, CdpConfig>>,

    /// GlobalPool from staking — read-only for staking_rise_sol_supply (ceiling denominator).
    pub global_pool: Box<Account<'info, GlobalPool>>,

    #[account(
        init,
        payer = borrower,
        space = CdpPosition::SIZE,
        seeds = [b"cdp_position", borrower.key().as_ref(), &[nonce]],
        bump
    )]
    pub position: Box<Account<'info, CdpPosition>>,

    #[account(
        mut,
        seeds = [b"collateral_config", collateral_config.mint.as_ref()],
        bump = collateral_config.bump
    )]
    pub collateral_config: Box<Account<'info, CollateralConfig>>,

    /// The collateral token mint.
    pub collateral_mint: Box<Account<'info, Mint>>,

    /// Borrower's collateral token account to transfer from.
    #[account(
        mut,
        constraint = borrower_collateral_account.mint == collateral_config.mint,
        constraint = borrower_collateral_account.owner == borrower.key()
    )]
    pub borrower_collateral_account: Box<Account<'info, TokenAccount>>,

    /// Protocol's collateral vault for this token type.
    #[account(
        mut,
        constraint = collateral_vault.mint == collateral_config.mint
    )]
    pub collateral_vault: Box<Account<'info, TokenAccount>>,

    /// CHECK: Pyth price feed for collateral token.
    pub pyth_price_feed: AccountInfo<'info>,

    /// CHECK: Pyth price feed for SOL/USD.
    pub sol_price_feed: AccountInfo<'info>,

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
    pub system_program: Program<'info, System>,

    /// Global borrow rewards config — updated with new total_cdp_debt.
    #[account(
        mut,
        seeds = [b"borrow_rewards_config"],
        bump = borrow_rewards_config.bump
    )]
    pub borrow_rewards_config: Box<Account<'info, BorrowRewardsConfig>>,

    /// Per-position borrow rewards tracker — initialized here.
    #[account(
        init,
        payer = borrower,
        space = BorrowRewards::SIZE,
        seeds = [b"borrow_rewards", position.key().as_ref()],
        bump
    )]
    pub borrow_rewards: Box<Account<'info, BorrowRewards>>,
}
