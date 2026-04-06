use anchor_lang::prelude::*;

#[error_code]
pub enum RewardsError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,

    #[msg("Gauge is not active")]
    GaugeNotActive,

    #[msg("No rewards to claim")]
    NoRewardsToClaim,

    #[msg("Math overflow occurred")]
    MathOverflow,

    #[msg("Epoch has not ended yet")]
    EpochNotEnded,

    #[msg("Unauthorized — only authority can call this")]
    Unauthorized,

    #[msg("Insufficient LP balance")]
    InsufficientBalance,

    #[msg("Weight basis points cannot exceed 10000")]
    InvalidWeight,

    #[msg("Gauge still has active LP deposits — all users must withdraw before closing")]
    GaugeHasActiveDeposits,
}
