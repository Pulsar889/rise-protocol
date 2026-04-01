use anchor_lang::prelude::*;

/// Global rewards configuration.
#[account]
pub struct RewardsConfig {
    /// Protocol authority.
    pub authority: Pubkey,

    /// RISE token mint.
    pub rise_mint: Pubkey,

    /// Total RISE emitted per epoch across all gauges.
    pub epoch_emissions: u64,

    /// Current epoch number.
    pub current_epoch: u64,

    /// Slot when current epoch started.
    pub epoch_start_slot: u64,

    /// Slots per epoch (~1 week).
    pub slots_per_epoch: u64,

    /// Total gauges created.
    pub gauge_count: u64,

    /// Bump seed.
    pub bump: u8,
}

impl RewardsConfig {
    pub const SIZE: usize = 8 + 32 + 32 + 8 + 8 + 8 + 8 + 8 + 1;
    pub const SLOTS_PER_EPOCH: u64 = 604_800; // ~1 week
}

/// A gauge represents a liquidity pool or activity that receives RISE emissions.
#[account]
pub struct Gauge {
    /// The pool or activity this gauge represents.
    pub pool: Pubkey,

    /// Gauge index.
    pub index: u64,

    /// Weight in basis points — set by veRISE gauge votes each epoch.
    pub weight_bps: u16,

    /// Whether this gauge is active.
    pub active: bool,

    /// Accumulated RISE per LP token (scaled by REWARD_SCALE).
    pub reward_per_token: u128,

    /// Total LP tokens deposited into this gauge.
    pub total_lp_deposited: u64,

    /// Last epoch this gauge was checkpointed.
    pub last_checkpoint_epoch: u64,

    /// Total RISE distributed to this gauge all time.
    pub total_distributed: u64,

    /// Emissions accumulated from epochs where total_lp_deposited == 0.
    /// Added to the next epoch's allocation when deposits exist.
    pub pending_emissions: u64,

    /// Bump seed.
    pub bump: u8,
}

impl Gauge {
    pub const SIZE: usize = 8 + 32 + 8 + 2 + 1 + 16 + 8 + 8 + 8 + 8 + 1;

    /// Scale factor for reward_per_token precision
    pub const REWARD_SCALE: u128 = 1_000_000_000_000;
}

/// Tracks a user's LP deposit in a specific gauge.
#[account]
pub struct UserStake {
    /// The wallet that owns this stake.
    pub owner: Pubkey,

    /// The gauge this stake belongs to.
    pub gauge: Pubkey,

    /// LP tokens deposited.
    pub lp_amount: u64,

    /// reward_per_token at last claim checkpoint.
    pub reward_debt: u128,

    /// Accumulated unclaimed RISE rewards.
    pub pending_rewards: u64,

    /// Bump seed.
    pub bump: u8,
}

impl UserStake {
    pub const SIZE: usize = 8 + 32 + 32 + 8 + 16 + 8 + 1;
}

/// Tracks per-epoch emissions for a gauge.
#[account]
pub struct EpochGaugeRewards {
    pub gauge: Pubkey,
    pub epoch: u64,
    pub rise_allocated: u64,
    pub rise_distributed: u64,
    pub bump: u8,
}

impl EpochGaugeRewards {
    pub const SIZE: usize = 8 + 32 + 8 + 8 + 8 + 1;
}
