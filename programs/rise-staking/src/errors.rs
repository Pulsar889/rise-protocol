use anchor_lang::prelude::*;

#[error_code]
pub enum StakingError {
    #[msg("The pool is currently paused")]
    PoolPaused,

    #[msg("Amount must be greater than zero")]
    ZeroAmount,

    #[msg("Insufficient liquid buffer for immediate redemption")]
    InsufficientLiquidity,

    #[msg("Exchange rate would be invalid after this operation")]
    InvalidExchangeRate,

    #[msg("Protocol fee basis points cannot exceed 10000")]
    InvalidFeeBps,

    #[msg("Liquid buffer target basis points cannot exceed 10000")]
    InvalidBufferBps,

    #[msg("Math overflow occurred")]
    MathOverflow,

    #[msg("Price oracle data is stale")]
    StaleOraclePrice,

    #[msg("Withdrawal ticket is not yet claimable — epoch delay has not passed")]
    UnstakeNotReady,

    #[msg("Caller is not authorized to perform this action")]
    Unauthorized,

    #[msg("GlobalPool has already been migrated to the latest size")]
    AlreadyMigrated,

    #[msg("No staking rewards to claim")]
    NoRewardsToClaim,

    #[msg("Invalid governance config account")]
    InvalidGovernanceConfig,
}
