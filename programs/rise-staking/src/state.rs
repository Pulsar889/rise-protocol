use anchor_lang::prelude::*;

#[account]
pub struct GlobalPool {
    pub authority: Pubkey,
    pub rise_sol_mint: Pubkey,
    pub total_sol_staked: u128,
    pub staking_rise_sol_supply: u128,
    pub exchange_rate: u128,
    pub last_rate_update_epoch: u64,
    pub liquid_buffer_lamports: u128,
    pub liquid_buffer_target_bps: u16,
    pub protocol_fee_bps: u16,
    pub paused: bool,
    /// SOL committed to pending withdrawal tickets, not yet claimed.
    pub pending_withdrawals_lamports: u128,
    pub bump: u8,
    /// CDP config PDA authorized to call notify_rise_sol_burned.
    /// Set once by authority via set_cdp_config after both programs are deployed.
    pub cdp_config_pubkey: Pubkey,
}

impl GlobalPool {
    pub const SIZE: usize = 8 + 32 + 32 + 16 + 16 + 16 + 8 + 16 + 2 + 2 + 1 + 16 + 1 + 32;
    pub const RATE_SCALE: u128 = 1_000_000_000;
    /// Epochs to wait before a withdrawal ticket can be claimed (~2 Solana epochs).
    pub const UNSTAKE_EPOCH_DELAY: u64 = 2;
    /// Minimum liquid buffer — 5% of deposits must stay uninvested.
    pub const MIN_LIQUID_BUFFER_BPS: u16 = 500;
    /// Default protocol staking fee — 5% of validator rewards.
    pub const DEFAULT_PROTOCOL_FEE_BPS: u16 = 500;

    pub fn sol_to_rise_sol(&self, sol_lamports: u64) -> Option<u64> {
        let rise_sol = (sol_lamports as u128)
            .checked_mul(Self::RATE_SCALE)?
            .checked_div(self.exchange_rate)?;
        u64::try_from(rise_sol).ok()
    }

    pub fn rise_sol_to_sol(&self, rise_sol_amount: u64) -> Option<u64> {
        let sol = (rise_sol_amount as u128)
            .checked_mul(self.exchange_rate)?
            .checked_div(Self::RATE_SCALE)?;
        u64::try_from(sol).ok()
    }
}

/// A queued unstake request. Created by `unstake_rise_sol`, claimed by `claim_unstake`
/// after UNSTAKE_EPOCH_DELAY epochs have passed.
#[account]
pub struct WithdrawalTicket {
    /// Wallet that requested the unstake.
    pub owner: Pubkey,
    /// SOL to return, locked in at the exchange rate when the ticket was created.
    pub sol_amount: u64,
    /// Earliest epoch the owner can call `claim_unstake`.
    pub claimable_epoch: u64,
    /// Nonce — allows multiple outstanding tickets per wallet.
    pub nonce: u8,
    /// Bump seed for PDA.
    pub bump: u8,
}

impl WithdrawalTicket {
    pub const SIZE: usize = 8  // discriminator
        + 32 // owner
        + 8  // sol_amount
        + 8  // claimable_epoch
        + 1  // nonce
        + 1; // bump
}

#[account]
pub struct ProtocolTreasury {
    /// Protocol authority.
    pub authority: Pubkey,

    /// Team salary wallet — receives team_fee_bps of all revenue.
    pub team_wallet: Pubkey,

    /// Team salary split in basis points. Default: 500 (5%).
    /// Adjustable by authority only.
    pub team_fee_bps: u16,

    /// Of the remaining 95%, what percentage goes to veRISE holders.
    /// Default: 5000 (50%). Adjustable by governance.
    pub verise_share_bps: u16,

    /// Total SOL accumulated in treasury reserve (retained portion).
    pub reserve_lamports: u128,

    /// Global revenue index for veRISE reward tracking.
    /// Increases each epoch by (epoch_revenue_for_verise / total_verise_supply).
    pub revenue_index: u128,

    /// Total SOL distributed to veRISE holders all time.
    pub total_distributed: u128,

    /// Last epoch fees were collected.
    pub last_collection_epoch: u64,

    /// Bump seed for PDA.
    pub bump: u8,
}

impl ProtocolTreasury {
    pub const SIZE: usize = 8   // discriminator
        + 32  // authority
        + 32  // team_wallet
        + 2   // team_fee_bps
        + 2   // verise_share_bps
        + 16  // reserve_lamports
        + 16  // revenue_index
        + 16  // total_distributed
        + 8   // last_collection_epoch
        + 1;  // bump

    /// Scale factor for revenue index precision
    pub const INDEX_SCALE: u128 = 1_000_000_000_000;

    /// Calculate team cut from total fees (team_fee_bps applied to total)
    pub fn team_cut(&self, total_fees: u64) -> Option<u64> {
        let cut = (total_fees as u128)
            .checked_mul(self.team_fee_bps as u128)?
            .checked_div(10_000)?;
        u64::try_from(cut).ok()
    }

    /// Calculate veRISE holder share from total fees (verise_share_bps applied to total)
    pub fn verise_cut(&self, total_fees: u64) -> Option<u64> {
        let cut = (total_fees as u128)
            .checked_mul(self.verise_share_bps as u128)?
            .checked_div(10_000)?;
        u64::try_from(cut).ok()
    }
}
