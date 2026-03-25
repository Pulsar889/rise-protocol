use anchor_lang::prelude::*;
use anchor_lang::system_program;
use crate::state::ProtocolTreasury;
use crate::errors::StakingError;

/// Tracks a veRISE holder's revenue claim state.
#[account]
pub struct RevenueClaimState {
    /// The wallet this claim state belongs to.
    pub owner: Pubkey,

    /// The revenue index at which rewards were last claimed.
    pub last_claimed_index: u128,

    /// Total SOL claimed all time by this wallet.
    pub total_claimed: u128,

    /// Bump seed for PDA.
    pub bump: u8,
}

impl RevenueClaimState {
    pub const SIZE: usize = 8 + 32 + 16 + 16 + 1;
}

pub fn handler(ctx: Context<ClaimRevenue>) -> Result<()> {
    let treasury = &ctx.accounts.treasury;
    let claim_state = &mut ctx.accounts.claim_state;

    // Calculate claimable amount based on veRISE balance and index delta
    // Full calculation requires veRISE balance from governance program
    // For v1 we use a simplified proportional claim
    let index_delta = treasury.revenue_index
        .checked_sub(claim_state.last_claimed_index)
        .ok_or(StakingError::MathOverflow)?;

    if index_delta == 0 {
        msg!("No rewards to claim");
        return Ok(());
    }

    // In v1: claimable = index_delta * verise_balance / INDEX_SCALE
    // verise_balance will come from governance program in full implementation
    // For now we store the balance passed in and verify it
    let verise_balance = ctx.accounts.verise_balance_account.lamports();

    let claimable = index_delta
        .checked_mul(verise_balance as u128)
        .ok_or(StakingError::MathOverflow)?
        .checked_div(ProtocolTreasury::INDEX_SCALE)
        .ok_or(StakingError::MathOverflow)? as u64;

    if claimable == 0 {
        msg!("Claimable amount is zero");
        claim_state.last_claimed_index = treasury.revenue_index;
        return Ok(());
    }

    // Transfer claimable SOL from treasury vault to user
    let vault_bump = ctx.bumps.treasury_vault;
    let seeds = &[b"treasury_vault".as_ref(), &[vault_bump]];
    let signer = &[&seeds[..]];

    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.system_program.to_account_info(),
        system_program::Transfer {
            from: ctx.accounts.treasury_vault.to_account_info(),
            to: ctx.accounts.user.to_account_info(),
        },
        signer,
    );
    system_program::transfer(cpi_ctx, claimable)?;

    // Update claim state
    claim_state.last_claimed_index = treasury.revenue_index;
    claim_state.total_claimed = claim_state
        .total_claimed
        .checked_add(claimable as u128)
        .ok_or(StakingError::MathOverflow)?;

    msg!("Claimed {} lamports in revenue", claimable);
    msg!("Total claimed all time: {}", claim_state.total_claimed);

    Ok(())
}

#[derive(Accounts)]
pub struct ClaimRevenue<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        init_if_needed,
        payer = user,
        space = RevenueClaimState::SIZE,
        seeds = [b"revenue_claim", user.key().as_ref()],
        bump
    )]
    pub claim_state: Account<'info, RevenueClaimState>,

    #[account(
        seeds = [b"protocol_treasury"],
        bump = treasury.bump
    )]
    pub treasury: Account<'info, ProtocolTreasury>,

    /// CHECK: Treasury SOL vault.
    #[account(
        mut,
        seeds = [b"treasury_vault"],
        bump
    )]
    pub treasury_vault: UncheckedAccount<'info>,

    /// CHECK: veRISE balance account — will be governance PDA in full implementation.
    pub verise_balance_account: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}
