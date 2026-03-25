use anchor_lang::prelude::*;

/// Global CDP configuration. One account for the entire program.
#[account]
pub struct CdpConfig {
    /// Protocol authority — can update debt ceiling.
    pub authority: Pubkey,

    /// Total riseSOL currently minted via CDP (principal only, not interest).
    pub cdp_rise_sol_minted: u128,

    /// Maximum CDP riseSOL as a multiple of staking riseSOL supply, in basis points.
    /// Default: 30000 = 3x. Adjustable by governance.
    pub debt_ceiling_multiplier_bps: u32,

    /// Bump seed for PDA.
    pub bump: u8,
}

impl CdpConfig {
    pub const SIZE: usize = 8  // discriminator
        + 32 // authority
        + 16 // cdp_rise_sol_minted
        + 4  // debt_ceiling_multiplier_bps
        + 1; // bump

    /// Default multiplier: 3x staking supply (30000 bps).
    pub const DEFAULT_DEBT_CEILING_MULTIPLIER_BPS: u32 = 30_000;

    /// Maximum size of a single CDP loan as a fraction of the total debt ceiling, in bps.
    /// 500 = 5%.
    pub const MAX_SINGLE_LOAN_BPS: u128 = 500;
}

/// Configuration for an accepted collateral type.
/// One account per collateral token, managed by governance.
#[account]
pub struct CollateralConfig {
    /// SPL token mint of the collateral asset.
    pub mint: Pubkey,

    /// Pyth price feed account for this asset.
    pub pyth_price_feed: Pubkey,

    /// Maximum loan-to-value in basis points (e.g. 8500 = 85%).
    pub max_ltv_bps: u16,

    /// Health factor threshold for liquidation.
    /// e.g. 9000 means liquidation triggered at 90% of max LTV.
    pub liquidation_threshold_bps: u16,

    /// Liquidation penalty in basis points. Default: 500 (5%).
    pub liquidation_penalty_bps: u16,

    /// Minimum annual interest rate in basis points (at 0% utilization).
    pub base_rate_bps: u32,

    /// Additional annual rate added at optimal utilization (the kink).
    /// e.g. 400 means rate goes from base_rate to base_rate+400 at optimal util.
    pub rate_slope1_bps: u32,

    /// Additional annual rate added from optimal utilization to 100%.
    /// Steep — discourages pushing past the kink.
    pub rate_slope2_bps: u32,

    /// Utilization at which rate slope steepens, in basis points.
    /// e.g. 8000 = 80% of debt ceiling.
    pub optimal_utilization_bps: u16,

    /// Slippage tolerance for DEX conversions in basis points.
    pub conversion_slippage_bps: u16,

    /// Whether new positions can be opened with this collateral.
    pub active: bool,

    /// Number of open positions using this collateral.
    pub total_positions: u64,

    /// Total collateral owed back to borrowers across all positions for this token.
    /// Incremented on deposit, decremented on return. May exceed vault balance
    /// if some collateral was seized for liquidity — the deficit is covered by
    /// converting from other vaults or SOL when borrowers repay.
    pub total_collateral_entitlements: u64,

    /// Bump seed for PDA.
    pub bump: u8,
}

/// Configuration for an accepted repayment token.
/// One account per accepted payment token (SOL, USDC, USDT, BTC, ETH).
/// Managed by protocol authority.
#[account]
pub struct PaymentConfig {
    /// SPL token mint for this payment token.
    /// Use anchor_lang::solana_program::system_program::ID as a sentinel
    /// to represent native SOL (no SPL mint).
    pub mint: Pubkey,

    /// Pyth price feed account for USD price of this token.
    pub pyth_price_feed: Pubkey,

    /// Whether this payment token is currently accepted.
    pub active: bool,

    /// Bump seed for PDA.
    pub bump: u8,
}

impl PaymentConfig {
    pub const SIZE: usize = 8  // discriminator
        + 32  // mint
        + 32  // pyth_price_feed
        + 1   // active
        + 1;  // bump

    /// Returns true if this config represents native SOL (not an SPL token).
    pub fn is_native_sol(&self) -> bool {
        self.mint == anchor_lang::solana_program::system_program::ID
    }
}

impl CollateralConfig {
    pub const SIZE: usize = 8   // discriminator
        + 32  // mint
        + 32  // pyth_price_feed
        + 2   // max_ltv_bps
        + 2   // liquidation_threshold_bps
        + 2   // liquidation_penalty_bps
        + 4   // base_rate_bps
        + 4   // rate_slope1_bps
        + 4   // rate_slope2_bps
        + 2   // optimal_utilization_bps
        + 2   // conversion_slippage_bps
        + 1   // active
        + 8   // total_positions
        + 8   // total_collateral_entitlements
        + 1;  // bump

    /// Scale factor for price calculations (6 decimal places for USD)
    pub const PRICE_SCALE: u128 = 1_000_000;

    /// Scale factor for rates (18 decimal places for precision)
    pub const RATE_SCALE: u128 = 1_000_000_000_000_000_000;

    /// Slots per year estimate (400ms per slot)
    pub const SLOTS_PER_YEAR: u128 = 78_840_000;
}

/// A single collateralized debt position.
/// One account per open CDP, owned by the borrower.
#[account]
pub struct CdpPosition {
    /// Wallet that opened this position.
    pub owner: Pubkey,

    /// Original collateral token mint.
    pub collateral_mint: Pubkey,

    /// Amount of collateral posted in original token units.
    pub collateral_amount_original: u64,

    /// USD value of collateral at last update (scaled by PRICE_SCALE).
    pub collateral_usd_value: u128,

    /// riseSOL minted to borrower (principal).
    pub rise_sol_debt_principal: u64,

    /// Accumulated interest in riseSOL units.
    pub interest_accrued: u64,

    /// Slot of last interest accrual.
    pub last_accrual_slot: u64,

    /// Last computed health factor (scaled by RATE_SCALE).
    /// Values below RATE_SCALE (1.0) are liquidatable.
    pub health_factor: u128,

    /// Slot when position was opened.
    pub opened_at_slot: u64,

    /// Position nonce — allows multiple positions per wallet.
    pub nonce: u8,

    /// Whether position is open.
    pub is_open: bool,

    /// SOL queued for excess withdrawal (in lamports).
    pub excess_withdrawal_queued: u64,

    /// Slot after which queued withdrawal can be processed.
    pub excess_withdrawal_available_slot: u64,

    /// Bump seed for PDA.
    pub bump: u8,
}

impl CdpPosition {
    pub const SIZE: usize = 8   // discriminator
        + 32  // owner
        + 32  // collateral_mint
        + 8   // collateral_amount_original
        + 16  // collateral_usd_value
        + 8   // rise_sol_debt_principal
        + 8   // interest_accrued
        + 8   // last_accrual_slot
        + 16  // health_factor
        + 8   // opened_at_slot
        + 1   // nonce
        + 1   // is_open
        + 8   // excess_withdrawal_queued
        + 8   // excess_withdrawal_available_slot
        + 1;  // bump

    /// Get total riseSOL owed (principal + interest)
    pub fn total_rise_sol_owed(&self) -> Option<u64> {
        self.rise_sol_debt_principal.checked_add(self.interest_accrued)
    }

    /// Compute health factor given current collateral and debt USD values.
    /// health_factor = (collateral_usd * liquidation_threshold_bps / 10000)
    ///                 / debt_usd
    /// Scaled by RATE_SCALE.
    pub fn compute_health_factor(
        collateral_usd: u128,
        debt_usd: u128,
        liquidation_threshold_bps: u16,
    ) -> Option<u128> {
        if debt_usd == 0 {
            return Some(u128::MAX); // infinite health if no debt
        }
        let adjusted_collateral = collateral_usd
            .checked_mul(liquidation_threshold_bps as u128)?
            .checked_div(10_000)?;
        adjusted_collateral
            .checked_mul(CollateralConfig::RATE_SCALE)?
            .checked_div(debt_usd)
    }
}
