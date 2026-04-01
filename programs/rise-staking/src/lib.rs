use anchor_lang::prelude::*;

declare_id!("BnQc6jJMT6mt3mvWuQFAd9vf2T2wWkAYD2uGjCXud6Lo");

pub mod state;
pub mod instructions;
pub mod errors;

use instructions::*;

#[program]
pub mod rise_staking {
    use super::*;

    /// Initialize the global staking pool.
    pub fn initialize_pool(
        ctx: Context<InitializePool>,
        protocol_fee_bps: u16,
        liquid_buffer_target_bps: u16,
    ) -> Result<()> {
        instructions::initialize_pool::handler(ctx, protocol_fee_bps, liquid_buffer_target_bps)
    }

    /// User deposits SOL and receives riseSOL.
    pub fn stake_sol(ctx: Context<StakeSol>, lamports: u64) -> Result<()> {
        instructions::stake_sol::handler(ctx, lamports)
    }

    /// User burns riseSOL and receives a WithdrawalTicket claimable after ~2 epochs.
    pub fn unstake_rise_sol(ctx: Context<UnstakeRiseSol>, rise_sol_amount: u64, nonce: u8) -> Result<()> {
        instructions::unstake_rise_sol::handler(ctx, rise_sol_amount, nonce)
    }

    /// User redeems a matured WithdrawalTicket and receives their SOL.
    pub fn claim_unstake(ctx: Context<ClaimUnstake>) -> Result<()> {
        instructions::claim_unstake::handler(ctx)
    }

    /// Crank: reads stake account balances and updates the exchange rate.
    /// stake_lamports_total: sum of all validator stake account balances (0 until delegation is built).
    pub fn update_exchange_rate(ctx: Context<UpdateExchangeRate>, stake_lamports_total: u64) -> Result<()> {
        instructions::update_exchange_rate::handler(ctx, stake_lamports_total)
    }

    /// Initialize the protocol treasury.
    pub fn initialize_treasury(
        ctx: Context<InitializeTreasury>,
        team_wallet: Pubkey,
        team_fee_bps: u16,
        verise_share_bps: u16,
    ) -> Result<()> {
        instructions::initialize_treasury::handler(ctx, team_wallet, team_fee_bps, verise_share_bps)
    }

    /// Crank: collect fees and distribute to team, treasury, and veRISE index.
    pub fn collect_fees(ctx: Context<CollectFees>) -> Result<()> {
        instructions::collect_fees::handler(ctx)
    }

    /// User claims accumulated veRISE revenue share.
    pub fn claim_revenue(ctx: Context<ClaimRevenue>) -> Result<()> {
        instructions::claim_revenue::handler(ctx)
    }

    /// Authority updates treasury configuration.
    pub fn update_treasury_config(
        ctx: Context<UpdateTreasuryConfig>,
        team_wallet: Option<Pubkey>,
        team_fee_bps: Option<u16>,
        verise_share_bps: Option<u16>,
    ) -> Result<()> {
        instructions::update_treasury_config::handler(
            ctx, team_wallet, team_fee_bps, verise_share_bps
        )
    }

    /// External programs (e.g. CDP) call this to register revenue they have
    /// already deposited into treasury_vault, updating the veRISE revenue index.
    pub fn register_external_revenue(
        ctx: Context<RegisterExternalRevenue>,
        verise_lamports: u64,
        reserve_lamports: u64,
    ) -> Result<()> {
        instructions::register_external_revenue::handler(ctx, verise_lamports, reserve_lamports)
    }

    /// Called after seized CDP collateral has been converted to SOL and deposited
    /// into pool_vault. Registers the amount as liquid buffer so withdrawal tickets
    /// can be paid out. Permissionless — verified by vault balance check.
    pub fn receive_cdp_liquidity(
        ctx: Context<ReceiveCdpLiquidity>,
        sol_amount: u64,
    ) -> Result<()> {
        instructions::receive_cdp_liquidity::handler(ctx, sol_amount)
    }

    /// Called by CDP program via CPI after burning riseSOL tokens as interest payment.
    /// Decrements staking_rise_sol_supply so the exchange rate rises to reflect the
    /// reduced supply — distributing the interest yield to remaining stakers.
    pub fn notify_rise_sol_burned(
        ctx: Context<NotifyRiseSolBurned>,
        amount: u64,
    ) -> Result<()> {
        instructions::notify_rise_sol_burned::handler(ctx, amount)
    }

    /// Authority-only: register the CDP config PDA to authorize CDP CPI calls.
    /// Call once after both programs are deployed.
    pub fn set_cdp_config(
        ctx: Context<SetCdpConfig>,
        cdp_config_pubkey: Pubkey,
    ) -> Result<()> {
        instructions::set_cdp_config::handler(ctx, cdp_config_pubkey)
    }

    /// One-time migration: reallocates GlobalPool to accommodate new APY tracking fields.
    /// Call once after deploying the updated program. Authority-only.
    pub fn migrate_global_pool(ctx: Context<MigrateGlobalPool>) -> Result<()> {
        instructions::migrate_global_pool::handler(ctx)
    }

    /// Called by the CDP program via CPI to fund a collateral buyback from the treasury.
    /// Transfers `shortfall_sol` lamports from treasury_vault to the CDP's WSOL buyback vault,
    /// which the CDP program then wraps and swaps → collateral tokens → borrower.
    pub fn withdraw_treasury_for_cdp_buyback(
        ctx: Context<WithdrawTreasuryForCdpBuyback>,
        shortfall_sol: u64,
    ) -> Result<()> {
        instructions::withdraw_treasury_for_cdp_buyback::handler(ctx, shortfall_sol)
    }

    /// Called by CDP program via CPI to mint riseSOL to a borrower.
    /// Authorized by cdp_config PDA signer. Does not affect staking_rise_sol_supply —
    /// CDP supply is tracked separately in cdp_rise_sol_minted on the CDP side.
    pub fn mint_for_cdp(ctx: Context<MintForCdp>, amount: u64) -> Result<()> {
        instructions::mint_for_cdp::handler(ctx, amount)
    }

    /// Called by CDP program after transferring fee revenue into pool_vault.
    /// Immediately credits the SOL to total_sol_staked and updates the exchange rate.
    /// Permissionless — integrity guaranteed by vault balance check.
    pub fn credit_staking_revenue(
        ctx: Context<CreditStakingRevenue>,
        amount: u64,
    ) -> Result<()> {
        instructions::credit_staking_revenue::handler(ctx, amount)
    }
}
