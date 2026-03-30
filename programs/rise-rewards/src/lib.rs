use anchor_lang::prelude::*;

declare_id!("8d3UidB3Ent4493deoozPYDC48XG2SRj7EdD7xW67uj8");

pub mod state;
pub mod instructions;
pub mod errors;

use instructions::*;

#[program]
pub mod rise_rewards {
    use super::*;

    /// Initialize the rewards program.
    pub fn initialize_rewards(
        ctx: Context<InitializeRewards>,
        epoch_emissions: u64,
    ) -> Result<()> {
        instructions::initialize_rewards::handler(ctx, epoch_emissions)
    }

    /// Create a new gauge for a liquidity pool.
    pub fn create_gauge(
        ctx: Context<CreateGauge>,
        pool: Pubkey,
    ) -> Result<()> {
        instructions::create_gauge::handler(ctx, pool)
    }

    /// Checkpoint a gauge — distribute epoch emissions based on gauge weight.
    pub fn checkpoint_gauge(ctx: Context<CheckpointGauge>) -> Result<()> {
        instructions::checkpoint_gauge::handler(ctx)
    }

    /// User deposits LP tokens into a gauge to earn RISE rewards.
    pub fn deposit_lp(
        ctx: Context<DepositLp>,
        amount: u64,
    ) -> Result<()> {
        instructions::deposit_lp::handler(ctx, amount)
    }

    /// User withdraws LP tokens from a gauge.
    pub fn withdraw_lp(
        ctx: Context<WithdrawLp>,
        amount: u64,
    ) -> Result<()> {
        instructions::withdraw_lp::handler(ctx, amount)
    }

    /// User claims accumulated RISE rewards from a gauge.
    pub fn claim_rewards(ctx: Context<ClaimRewards>) -> Result<()> {
        instructions::claim_rewards::handler(ctx)
    }

    /// Authority sets RISE emissions for the next epoch.
    pub fn set_epoch_emissions(
        ctx: Context<SetEpochEmissions>,
        epoch_emissions: u64,
    ) -> Result<()> {
        instructions::set_epoch_emissions::handler(ctx, epoch_emissions)
    }

    /// Close the rewards config, reclaiming rent. Authority only.
    /// Use this to reset and re-initialize with a new RISE mint.
    pub fn close_rewards_config(ctx: Context<CloseRewardsConfig>) -> Result<()> {
        instructions::close_rewards_config::handler(ctx)
    }

    /// Create the RISE vault that backs LP gauge reward payouts. Authority only.
    /// Call once after initialize_rewards.
    pub fn initialize_rewards_vault(ctx: Context<InitializeRewardsVault>) -> Result<()> {
        instructions::initialize_rewards_vault::handler(ctx)
    }
}
