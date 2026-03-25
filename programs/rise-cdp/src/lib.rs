use anchor_lang::prelude::*;

declare_id!("3snPJTuZP9XHNciH7Q5KZzsvk2doxpuoYqWXf8JofEPR");

pub mod state;
pub mod instructions;
pub mod errors;

use instructions::*;

#[program]
pub mod rise_cdp {
    use super::*;

    /// Initialize a new accepted collateral type. Authority only.
    pub fn initialize_collateral_config(
        ctx: Context<InitializeCollateralConfig>,
        max_ltv_bps: u16,
        liquidation_threshold_bps: u16,
        liquidation_penalty_bps: u16,
        base_rate_bps: u32,
        rate_slope1_bps: u32,
        rate_slope2_bps: u32,
        optimal_utilization_bps: u16,
        conversion_slippage_bps: u16,
    ) -> Result<()> {
        instructions::initialize_collateral_config::handler(
            ctx, max_ltv_bps, liquidation_threshold_bps, liquidation_penalty_bps,
            base_rate_bps, rate_slope1_bps, rate_slope2_bps,
            optimal_utilization_bps, conversion_slippage_bps,
        )
    }

    /// Initialize the token vault for a collateral type. Authority only.
    pub fn initialize_collateral_vault(
        ctx: Context<InitializeCollateralVault>,
    ) -> Result<()> {
        instructions::initialize_collateral_vault::handler(ctx)
    }

    /// Update parameters on an existing collateral config. Authority only.
    pub fn update_collateral_config(
        ctx: Context<UpdateCollateralConfig>,
        max_ltv_bps: Option<u16>,
        liquidation_threshold_bps: Option<u16>,
        liquidation_penalty_bps: Option<u16>,
        base_rate_bps: Option<u32>,
        rate_slope1_bps: Option<u32>,
        rate_slope2_bps: Option<u32>,
        optimal_utilization_bps: Option<u16>,
        conversion_slippage_bps: Option<u16>,
        active: Option<bool>,
    ) -> Result<()> {
        instructions::update_collateral_config::handler(
            ctx, max_ltv_bps, liquidation_threshold_bps, liquidation_penalty_bps,
            base_rate_bps, rate_slope1_bps, rate_slope2_bps,
            optimal_utilization_bps, conversion_slippage_bps, active,
        )
    }

    /// Open a new CDP position.
    pub fn open_position(
        ctx: Context<OpenPosition>,
        collateral_amount: u64,
        rise_sol_to_mint: u64,
        nonce: u8,
    ) -> Result<()> {
        instructions::open_position::handler(ctx, collateral_amount, rise_sol_to_mint, nonce)
    }

    /// Close a CDP position.
    pub fn close_position(ctx: Context<ClosePosition>) -> Result<()> {
        instructions::close_position::handler(ctx)
    }

    /// Add collateral to an existing position.
    pub fn add_collateral(ctx: Context<AddCollateral>, amount: u64) -> Result<()> {
        instructions::add_collateral::handler(ctx, amount)
    }

    /// Queue an excess collateral withdrawal.
    pub fn withdraw_excess(ctx: Context<WithdrawExcess>, amount: u64) -> Result<()> {
        instructions::withdraw_excess::handler(ctx, amount)
    }

    /// Liquidate an unhealthy position.
    pub fn liquidate(ctx: Context<Liquidate>) -> Result<()> {
        instructions::liquidate::handler(ctx)
    }

    /// Crank: accrue interest on a position.
    pub fn accrue_interest(ctx: Context<AccrueInterest>) -> Result<()> {
        instructions::accrue_interest::handler(ctx)
    }

    /// Initialize a payment token config (SOL, USDC, USDT, BTC, ETH). Authority only.
    pub fn initialize_payment_config(ctx: Context<InitializePaymentConfig>) -> Result<()> {
        instructions::initialize_payment_config::handler(ctx)
    }

    /// Repay all or part of a CDP debt using SOL or an accepted SPL token.
    pub fn repay_debt(ctx: Context<RepayDebt>, payment_amount: u64) -> Result<()> {
        instructions::repay_debt::handler(ctx, payment_amount)
    }

    /// Mint additional riseSOL against an existing open position (subject to max LTV).
    pub fn borrow_more(ctx: Context<BorrowMore>, additional_rise_sol: u64) -> Result<()> {
        instructions::borrow_more::handler(ctx, additional_rise_sol)
    }

    /// Sweep accumulated CDP interest fees and distribute via 5/47.5/47.5 split.
    pub fn collect_cdp_fees(ctx: Context<CollectCdpFees>) -> Result<()> {
        instructions::collect_cdp_fees::handler(ctx)
    }

    /// Repay all or part of a CDP debt by burning riseSOL tokens directly (1:1).
    pub fn repay_debt_rise_sol(ctx: Context<RepayDebtRiseSol>, payment_rise_sol: u64) -> Result<()> {
        instructions::repay_debt_rise_sol::handler(ctx, payment_rise_sol)
    }

    /// Initialize the global CDP config (debt ceiling). Authority only.
    pub fn initialize_cdp_config(
        ctx: Context<InitializeCdpConfig>,
        debt_ceiling_multiplier_bps: u32,
    ) -> Result<()> {
        instructions::initialize_cdp_config::handler(ctx, debt_ceiling_multiplier_bps)
    }

    /// Seize collateral to cover a staking pool liquidity shortfall. Permissionless.
    pub fn redeem_collateral_for_liquidity(
        ctx: Context<RedeemCollateralForLiquidity>,
        amount: u64,
    ) -> Result<()> {
        instructions::redeem_collateral_for_liquidity::handler(ctx, amount)
    }

    /// Update the global CDP debt ceiling multiplier. Authority or governance only.
    pub fn update_debt_ceiling(
        ctx: Context<UpdateDebtCeiling>,
        multiplier_bps: u32,
    ) -> Result<()> {
        instructions::update_debt_ceiling::handler(ctx, multiplier_bps)
    }
}
