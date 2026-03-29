use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::state::{BorrowRewards, BorrowRewardsConfig, CdpPosition};
use crate::errors::CdpError;

/// Claim accumulated RISE borrow rewards for a CDP position.
///
/// Settles any newly-accrued rewards since the last checkpoint, then transfers
/// the full pending balance from the rewards vault to the borrower's RISE ATA.
pub fn handler(ctx: Context<ClaimBorrowRewards>) -> Result<()> {
    let reward_per_token = ctx.accounts.borrow_rewards_config.reward_per_token;
    let current_debt = ctx.accounts.position.rise_sol_debt_principal;

    let rewards = &mut ctx.accounts.borrow_rewards;

    // Settle any rewards that have accrued since the last update.
    rewards.settle(reward_per_token, current_debt)?;

    let total_claimable = rewards.pending_rewards;
    require!(total_claimable > 0, CdpError::NoRewardsToClaim);

    // Transfer from rewards_vault (authority = borrow_rewards_config PDA).
    let config_bump = ctx.accounts.borrow_rewards_config.bump;
    let seeds = &[b"borrow_rewards_config".as_ref(), &[config_bump]];
    let signer = &[&seeds[..]];

    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.token_program.to_account_info(),
        Transfer {
            from: ctx.accounts.rewards_vault.to_account_info(),
            to: ctx.accounts.borrower_rise_account.to_account_info(),
            authority: ctx.accounts.borrow_rewards_config.to_account_info(),
        },
        signer,
    );
    token::transfer(cpi_ctx, total_claimable)?;

    // Update accounting.
    rewards.pending_rewards = 0;
    rewards.total_claimed = rewards.total_claimed
        .checked_add(total_claimable)
        .ok_or(CdpError::MathOverflow)?;
    rewards.last_checkpoint_slot = Clock::get()?.slot;

    // Re-sync reward_debt against current debt and updated reward_per_token.
    rewards.sync_debt(reward_per_token, current_debt)?;

    msg!("Claimed {} RISE borrow rewards", total_claimable);
    msg!("Lifetime claimed: {}", rewards.total_claimed);

    Ok(())
}

#[derive(Accounts)]
#[instruction(position_nonce: u8)]
pub struct ClaimBorrowRewards<'info> {
    #[account(mut)]
    pub borrower: Signer<'info>,

    #[account(
        seeds = [b"cdp_position", borrower.key().as_ref(), &[position_nonce]],
        bump = position.bump,
        constraint = position.owner == borrower.key()
    )]
    pub position: Account<'info, CdpPosition>,

    #[account(
        mut,
        seeds = [b"borrow_rewards", position.key().as_ref()],
        bump = borrow_rewards.bump,
        constraint = borrow_rewards.owner == borrower.key(),
        constraint = borrow_rewards.position == position.key()
    )]
    pub borrow_rewards: Account<'info, BorrowRewards>,

    #[account(
        seeds = [b"borrow_rewards_config"],
        bump = borrow_rewards_config.bump
    )]
    pub borrow_rewards_config: Account<'info, BorrowRewardsConfig>,

    #[account(
        mut,
        seeds = [b"borrow_rewards_vault"],
        bump,
        constraint = rewards_vault.mint == borrow_rewards_config.rise_mint
    )]
    pub rewards_vault: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = borrower_rise_account.mint == borrow_rewards_config.rise_mint,
        constraint = borrower_rise_account.owner == borrower.key()
    )]
    pub borrower_rise_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}
