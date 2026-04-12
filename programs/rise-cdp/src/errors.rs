use anchor_lang::prelude::*;

#[error_code]
pub enum CdpError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,

    #[msg("This collateral type is not accepted")]
    CollateralNotAccepted,

    #[msg("Borrow amount exceeds maximum LTV")]
    ExceedsMaxLtv,

    #[msg("Position is healthy and cannot be liquidated")]
    PositionHealthy,

    #[msg("Position is unhealthy and must be liquidated before withdrawal")]
    PositionUnhealthy,

    #[msg("Insufficient excess collateral to withdraw requested amount")]
    InsufficientExcess,

    #[msg("Math overflow occurred")]
    MathOverflow,

    #[msg("Oracle price is stale")]
    StaleOraclePrice,

    #[msg("Oracle price is invalid")]
    InvalidOraclePrice,

    #[msg("Position is already closed")]
    PositionClosed,

    #[msg("Interest accrual is up to date")]
    InterestUpToDate,

    #[msg("Insufficient riseSOL balance to cover outstanding debt")]
    InsufficientRepaymentBalance,

    #[msg("The pool is currently paused")]
    PoolPaused,

    #[msg("This payment token is not active")]
    PaymentConfigInactive,

    #[msg("No CDP fees accumulated to collect")]
    NoCdpFeesToCollect,

    #[msg("Minting would exceed the global CDP debt ceiling")]
    DebtCeilingExceeded,

    #[msg("Loan amount exceeds the 5% single-loan cap")]
    ExceedsSingleLoanCap,

    #[msg("Liquidity redemption not needed — liquid buffer covers pending withdrawals")]
    LiquidityRedemptionNotNeeded,

    #[msg("No borrow rewards available to claim")]
    NoRewardsToClaim,

    #[msg("Borrow rewards not yet initialized")]
    BorrowRewardsNotInitialized,

    #[msg("Oracle price confidence interval is too wide")]
    InsufficientPriceConfidence,

    #[msg("Collateral shortfall exists — provide a buyback route to complete repayment")]
    CollateralShortfall,

    #[msg("Caller is not authorized for this operation")]
    Unauthorized,

    #[msg("Price feed account does not match the one registered for this collateral")]
    WrongPriceFeed,
}
