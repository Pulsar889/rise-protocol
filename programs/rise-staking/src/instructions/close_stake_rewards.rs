use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, CloseAccount};
use crate::state::{StakeRewardsConfig, GlobalPool};
use crate::errors::StakingError;

/// Closes the stake rewards config and vault, reclaiming rent.
/// Transfers any remaining RISE tokens back to the authority before closing.
/// Authority only — call this before re-initializing with a new RISE mint.
pub fn handler(ctx: Context<CloseStakeRewards>) -> Result<()> {
    let config_bump = ctx.accounts.stake_rewards_config.bump;
    let seeds: &[&[u8]] = &[b"stake_rewards_config", &[config_bump]];
    let signer = &[seeds];

    // Return any remaining RISE to the authority wallet.
    let remaining = ctx.accounts.rewards_vault.amount;
    if remaining > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from:      ctx.accounts.rewards_vault.to_account_info(),
                    to:        ctx.accounts.authority_rise_account.to_account_info(),
                    authority: ctx.accounts.stake_rewards_config.to_account_info(),
                },
                signer,
            ),
            remaining,
        )?;
        msg!("Returned {} RISE tokens to authority", remaining);
    }

    // Close the token vault — rent goes to authority.
    token::close_account(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account:     ctx.accounts.rewards_vault.to_account_info(),
                destination: ctx.accounts.authority.to_account_info(),
                authority:   ctx.accounts.stake_rewards_config.to_account_info(),
            },
            signer,
        ),
    )?;

    // stake_rewards_config is closed by the `close = authority` constraint.
    msg!("Stake rewards config and vault closed successfully");
    Ok(())
}

#[derive(Accounts)]
pub struct CloseStakeRewards<'info> {
    #[account(
        mut,
        constraint = authority.key() == pool.authority @ StakingError::Unauthorized
    )]
    pub authority: Signer<'info>,

    #[account(
        seeds = [b"global_pool"],
        bump = pool.bump
    )]
    pub pool: Account<'info, GlobalPool>,

    #[account(
        mut,
        seeds = [b"stake_rewards_config"],
        bump = stake_rewards_config.bump,
        close = authority,
    )]
    pub stake_rewards_config: Account<'info, StakeRewardsConfig>,

    #[account(
        mut,
        seeds = [b"stake_rewards_vault"],
        bump,
        constraint = rewards_vault.mint == stake_rewards_config.rise_mint @ StakingError::Unauthorized,
    )]
    pub rewards_vault: Account<'info, TokenAccount>,

    /// Authority's RISE token account — receives the returned tokens.
    #[account(
        mut,
        constraint = authority_rise_account.mint == stake_rewards_config.rise_mint @ StakingError::Unauthorized,
        constraint = authority_rise_account.owner == authority.key() @ StakingError::Unauthorized,
    )]
    pub authority_rise_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}
