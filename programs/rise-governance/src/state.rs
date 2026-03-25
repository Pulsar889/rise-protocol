use anchor_lang::prelude::*;

/// A single gauge allocation entry.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Default)]
pub struct GaugeAllocation {
    pub pool: Pubkey,
    pub weight_bps: u16,
}

/// Global governance configuration.
#[account]
pub struct GovernanceConfig {
    pub authority: Pubkey,
    pub rise_mint: Pubkey,
    pub total_verise: u128,
    pub min_lock_slots: u64,
    pub max_lock_slots: u64,
    pub proposal_threshold: u64,
    pub voting_period_slots: u64,
    pub timelock_slots: u64,
    pub quorum_bps: u16,
    pub proposal_count: u64,
    /// Sequential counter used to name veRISE Lock NFTs (#1, #2, …)
    pub lock_count: u64,
    pub bump: u8,
}

impl GovernanceConfig {
    pub const SIZE: usize = 8 + 32 + 32 + 16 + 8 + 8 + 8 + 8 + 8 + 2 + 8 + 8 + 1;
    pub const SLOTS_PER_WEEK: u64 = 604_800;
    pub const SLOTS_PER_YEAR: u64 = 78_840_000;
    pub const MAX_LOCK_SLOTS: u64 = 4 * 78_840_000;
    pub const VERISE_SCALE: u128 = 1_000_000_000;

    pub fn calculate_verise(rise_amount: u64, lock_slots: u64) -> Option<u64> {
        let verise = (rise_amount as u128)
            .checked_mul(lock_slots as u128)?
            .checked_div(Self::MAX_LOCK_SLOTS as u128)?;
        u64::try_from(verise).ok()
    }
}

#[account]
pub struct VeLock {
    pub owner: Pubkey,
    pub rise_locked: u64,
    pub verise_amount: u64,
    pub lock_start_slot: u64,
    pub lock_end_slot: u64,
    pub last_revenue_index: u128,
    pub total_revenue_claimed: u64,
    /// The mint address of the veRISE Lock NFT issued for this position.
    pub nft_mint: Pubkey,
    /// Sequential lock number used in the NFT name ("veRISE Lock #N").
    pub lock_number: u64,
    pub nonce: u8,
    pub bump: u8,
}

impl VeLock {
    pub const SIZE: usize = 8 + 32 + 8 + 8 + 8 + 8 + 16 + 8 + 32 + 8 + 1 + 1;

    pub fn current_verise(&self, current_slot: u64) -> u64 {
        if current_slot >= self.lock_end_slot {
            return 0;
        }
        let remaining_slots = self.lock_end_slot - current_slot;
        let total_slots = self.lock_end_slot - self.lock_start_slot;
        if total_slots == 0 {
            return 0;
        }
        ((self.verise_amount as u128)
            .saturating_mul(remaining_slots as u128)
            / (total_slots as u128)) as u64
    }
}

#[account]
pub struct Proposal {
    pub proposer: Pubkey,
    pub description: [u8; 128],
    pub target_program: Pubkey,
    pub voting_end_slot: u64,
    pub execution_slot: u64,
    pub votes_for: u128,
    pub votes_against: u128,
    pub executed: bool,
    pub index: u64,
    pub bump: u8,
}

impl Proposal {
    pub const SIZE: usize = 8 + 32 + 128 + 32 + 8 + 8 + 16 + 16 + 1 + 8 + 1;

    pub fn is_passed(&self, total_verise: u128, quorum_bps: u16) -> bool {
        let total_votes = self.votes_for + self.votes_against;
        let quorum = total_verise
            .saturating_mul(quorum_bps as u128)
            / 10_000;
        total_votes >= quorum && self.votes_for > self.votes_against
    }
}

#[account]
pub struct VoteRecord {
    pub voter: Pubkey,
    pub proposal: Pubkey,
    pub verise_at_vote: u64,
    pub vote_for: bool,
    pub bump: u8,
}

impl VoteRecord {
    pub const SIZE: usize = 8 + 32 + 32 + 8 + 1 + 1;
}

#[account]
pub struct GaugeVote {
    pub owner: Pubkey,
    pub epoch: u64,
    pub gauges: [GaugeAllocation; 8],
    pub bump: u8,
}

impl GaugeVote {
    pub const SIZE: usize = 8 + 32 + 8 + (34 * 8) + 1;
}
