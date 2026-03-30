use anchor_lang::prelude::*;

declare_id!("3snPJTuZP9XHNciH7Q5KZzsvk2doxpuoYqWXf8JofEPR");

pub mod state;
pub mod instructions;
pub mod errors;
pub mod jupiter;
pub mod pyth;

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
    /// `route_plan_data` is Borsh-serialized `Vec<RoutePlanStep>` from the Jupiter quote API.
    /// See `src/jupiter.rs` for encoding notes.
    pub fn liquidate(
        ctx: Context<Liquidate>,
        route_plan_data: Vec<u8>,
        quoted_out_amount: u64,
        slippage_bps: u16,
    ) -> Result<()> {
        instructions::liquidate::handler(ctx, route_plan_data, quoted_out_amount, slippage_bps)
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
    /// For SPL token payments, `route_plan_data` is Borsh-serialized `Vec<RoutePlanStep>` from
    /// the Jupiter quote API; it is swapped to SOL on-chain. Pass empty / 0 for native SOL.
    pub fn repay_debt(
        ctx: Context<RepayDebt>,
        payment_amount: u64,
        route_plan_data: Vec<u8>,
        quoted_out_amount: u64,
        slippage_bps: u16,
    ) -> Result<()> {
        instructions::repay_debt::handler(ctx, payment_amount, route_plan_data, quoted_out_amount, slippage_bps)
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
    /// `shortfall_route_plan_data` is used on full repayment when seized collateral must be
    /// bought back via Jupiter. Pass empty / 0 when no shortfall is expected (the common case).
    pub fn repay_debt_rise_sol(
        ctx: Context<RepayDebtRiseSol>,
        payment_rise_sol: u64,
        shortfall_route_plan_data: Vec<u8>,
        shortfall_quoted_out: u64,
        shortfall_slippage_bps: u16,
    ) -> Result<()> {
        instructions::repay_debt_rise_sol::handler(
            ctx, payment_rise_sol, shortfall_route_plan_data, shortfall_quoted_out, shortfall_slippage_bps,
        )
    }

    /// Initialize the global CDP config (debt ceiling). Authority only.
    pub fn initialize_cdp_config(
        ctx: Context<InitializeCdpConfig>,
        debt_ceiling_multiplier_bps: u32,
    ) -> Result<()> {
        instructions::initialize_cdp_config::handler(ctx, debt_ceiling_multiplier_bps)
    }

    /// Seize collateral to cover a staking pool liquidity shortfall. Permissionless.
    /// `route_plan_data` is Borsh-serialized `Vec<RoutePlanStep>` from the Jupiter quote API;
    /// the seized collateral is swapped → SOL and deposited into pool_vault as liquid buffer.
    pub fn redeem_collateral_for_liquidity(
        ctx: Context<RedeemCollateralForLiquidity>,
        amount: u64,
        route_plan_data: Vec<u8>,
        quoted_out_amount: u64,
        slippage_bps: u16,
    ) -> Result<()> {
        instructions::redeem_collateral_for_liquidity::handler(ctx, amount, route_plan_data, quoted_out_amount, slippage_bps)
    }

    /// Update the global CDP debt ceiling multiplier. Authority or governance only.
    pub fn update_debt_ceiling(
        ctx: Context<UpdateDebtCeiling>,
        multiplier_bps: u32,
    ) -> Result<()> {
        instructions::update_debt_ceiling::handler(ctx, multiplier_bps)
    }

    /// Initialize the borrow rewards config and vault. Authority only.
    pub fn initialize_borrow_rewards(
        ctx: Context<InitializeBorrowRewards>,
        epoch_emissions: u64,
        slots_per_epoch: u64,
    ) -> Result<()> {
        instructions::initialize_borrow_rewards::handler(ctx, epoch_emissions, slots_per_epoch)
    }

    /// Permissionless crank — advance the global reward_per_token accumulator.
    pub fn checkpoint_borrow_rewards(ctx: Context<CheckpointBorrowRewards>) -> Result<()> {
        instructions::checkpoint_borrow_rewards::handler(ctx)
    }

    /// Claim accumulated RISE borrow rewards for a CDP position.
    pub fn claim_borrow_rewards(
        ctx: Context<ClaimBorrowRewards>,
        position_nonce: u8,
    ) -> Result<()> {
        instructions::claim_borrow_rewards::handler(ctx)
    }

    /// Close borrow_rewards_config and borrow_rewards_vault. Authority only.
    /// Burns any remaining tokens in the vault before closing. Use this to
    /// reset and re-initialize with a new RISE mint.
    pub fn close_borrow_rewards(ctx: Context<CloseBorrowRewards>) -> Result<()> {
        instructions::close_borrow_rewards::handler(ctx)
    }
}
