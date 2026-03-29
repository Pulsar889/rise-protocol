use anchor_lang::prelude::*;
use anchor_spl::token::{Token, TokenAccount, Mint};
use crate::state::{BorrowRewardsConfig, CdpConfig};
use crate::errors::CdpError;

pub fn handler(
    ctx: Context<InitializeBorrowRewards>,
    epoch_emissions: u64,
    slots_per_epoch: u64,
) -> Result<()> {
    require!(epoch_emissions > 0, CdpError::ZeroAmount);
    require!(slots_per_epoch > 0, CdpError::ZeroAmount);

    let config = &mut ctx.accounts.borrow_rewards_config;
    let current_slot = Clock::get()?.slot;

    config.authority = ctx.accounts.authority.key();
    config.rise_mint = ctx.accounts.rise_mint.key();
    config.rewards_vault = ctx.accounts.rewards_vault.key();
    config.reward_per_token = 0;
    config.epoch_emissions = epoch_emissions;
    config.slots_per_epoch = slots_per_epoch;
    config.total_cdp_debt = 0;
    config.last_checkpoint_slot = current_slot;
    config.bump = ctx.bumps.borrow_rewards_config;

    msg!("Borrow rewards initialized");
    msg!("RISE mint: {}", config.rise_mint);
    msg!("Epoch emissions: {}", epoch_emissions);
    msg!("Slots per epoch: {}", slots_per_epoch);

    Ok(())
}

#[derive(Accounts)]
pub struct InitializeBorrowRewards<'info> {
    #[account(
        mut,
        constraint = authority.key() == cdp_config.authority @ CdpError::BorrowRewardsNotInitialized
    )]
    pub authority: Signer<'info>,

    /// Global CDP config — used to verify authority.
    #[account(
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Account<'info, CdpConfig>,

    #[account(
        init,
        payer = authority,
        space = BorrowRewardsConfig::SIZE,
        seeds = [b"borrow_rewards_config"],
        bump
    )]
    pub borrow_rewards_config: Account<'info, BorrowRewardsConfig>,

    /// Token account PDA that will hold RISE for borrow rewards.
    /// Authority is the borrow_rewards_config PDA itself.
    #[account(
        init,
        payer = authority,
        token::mint = rise_mint,
        token::authority = borrow_rewards_config,
        seeds = [b"borrow_rewards_vault"],
        bump
    )]
    pub rewards_vault: Account<'info, TokenAccount>,

    pub rise_mint: Account<'info, Mint>,

    /// Must be the standard SPL Token program — Token-2022 is not accepted.
    #[account(address = Token::id())]
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}
