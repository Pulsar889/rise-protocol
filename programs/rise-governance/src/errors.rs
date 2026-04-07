use anchor_lang::prelude::*;

#[error_code]
pub enum GovernanceError {
    #[msg("Lock duration is too short — minimum 1 week")]
    LockTooShort,

    #[msg("Lock duration is too long — maximum 4 years")]
    LockTooLong,

    #[msg("Lock has not yet expired")]
    LockNotExpired,

    #[msg("Lock is already expired")]
    LockExpired,

    #[msg("Amount must be greater than zero")]
    ZeroAmount,

    #[msg("Gauge votes must sum to exactly 10000 basis points")]
    InvalidGaugeWeights,

    #[msg("Proposal voting period has ended")]
    VotingEnded,

    #[msg("Proposal voting period has not ended yet")]
    VotingNotEnded,

    #[msg("Proposal has already been executed")]
    AlreadyExecuted,

    #[msg("Proposal did not pass")]
    ProposalFailed,

    #[msg("Timelock has not elapsed")]
    TimelockNotElapsed,

    #[msg("Insufficient veRISE balance to create proposal")]
    InsufficientVeRise,

    #[msg("Math overflow occurred")]
    MathOverflow,

    #[msg("No rewards to claim")]
    NoRewardsToClaim,

    #[msg("Already voted on this proposal")]
    AlreadyVoted,

    #[msg("Invalid governance config parameter")]
    InvalidConfig,

    #[msg("Vault did not receive the expected token amount")]
    TransferAmountMismatch,
}
