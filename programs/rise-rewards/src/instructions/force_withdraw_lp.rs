use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use crate::state::{RewardsConfig, Gauge, UserStake};
use crate::errors::RewardsError;

/// Authority-only: force-refund a depositor's LP tokens and any pending RISE rewards.
/// Use this to clear out a gauge before closing it.
/// The depositor does not need to sign — authority acts on their behalf.
pub fn handler(ctx: Context<ForceWithdrawLp>) -> Result<()> {
    let gauge      = &mut ctx.accounts.gauge;
    let stake      = &ctx.accounts.user_stake;
    let lp_amount  = stake.lp_amount;

    require!(lp_amount > 0, RewardsError::ZeroAmount);

    // ── Settle pending RISE rewards ──────────────────────────────────────────
    let newly_accrued = (lp_amount as u128)
        .checked_mul(gauge.reward_per_token)
        .ok_or(RewardsError::MathOverflow)?
        .checked_div(Gauge::REWARD_SCALE)
        .ok_or(RewardsError::MathOverflow)?
        .saturating_sub(stake.reward_debt) as u64;

    let total_claimable = stake.pending_rewards
        .checked_add(newly_accrued)
        .ok_or(RewardsError::MathOverflow)?;

    // ── Transfer LP tokens back to depositor ─────────────────────────────────
    let pool_key     = gauge.pool.clone();
    let vault_bump   = ctx.bumps.gauge_lp_vault;
    let vault_seeds  = &[b"gauge_lp_vault".as_ref(), pool_key.as_ref(), &[vault_bump]];
    let vault_signer = &[&vault_seeds[..]];

    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from:      ctx.accounts.gauge_lp_vault.to_account_info(),
                to:        ctx.accounts.user_lp_account.to_account_info(),
                authority: ctx.accounts.gauge_lp_vault.to_account_info(),
            },
            vault_signer,
        ),
        lp_amount,
    )?;

    // ── Transfer RISE rewards to depositor ───────────────────────────────────
    // Cap at vault balance so a depleted rewards vault never blocks LP withdrawal.
    let claimable = total_claimable.min(ctx.accounts.rewards_vault.amount);
    if claimable > 0 {
        let config_bump   = ctx.accounts.config.bump;
        let config_seeds  = &[b"rewards_config".as_ref(), &[config_bump]];
        let config_signer = &[&config_seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from:      ctx.accounts.rewards_vault.to_account_info(),
                    to:        ctx.accounts.user_rise_account.to_account_info(),
                    authority: ctx.accounts.config.to_account_info(),
                },
                config_signer,
            ),
            claimable,
        )?;
    }

    // ── Update gauge total ───────────────────────────────────────────────────
    gauge.total_lp_deposited = gauge.total_lp_deposited
        .checked_sub(lp_amount)
        .ok_or(RewardsError::MathOverflow)?;

    msg!(
        "Force withdrew {} LP tokens and {} RISE rewards for {}",
        lp_amount,
        total_claimable,
        ctx.accounts.user_stake.owner
    );

    Ok(())
}

#[derive(Accounts)]
pub struct ForceWithdrawLp<'info> {
    #[account(
        constraint = authority.key() == config.authority @ RewardsError::Unauthorized
    )]
    pub authority: Signer<'info>,

    #[account(
        seeds = [b"rewards_config"],
        bump = config.bump,
    )]
    pub config: Box<Account<'info, RewardsConfig>>,

    #[account(
        mut,
        seeds = [b"gauge", gauge.pool.as_ref()],
        bump = gauge.bump,
    )]
    pub gauge: Box<Account<'info, Gauge>>,

    #[account(
        mut,
        seeds = [b"user_stake", user_stake.owner.as_ref(), gauge.key().as_ref()],
        bump = user_stake.bump,
        close = depositor
    )]
    pub user_stake: Box<Account<'info, UserStake>>,

    /// CHECK: The depositor — receives LP tokens, RISE rewards, and rent from user_stake close.
    /// Verified via user_stake.owner.
    #[account(
        mut,
        constraint = depositor.key() == user_stake.owner @ RewardsError::Unauthorized
    )]
    pub depositor: UncheckedAccount<'info>,

    /// Depositor's LP token account — receives the returned LP tokens.
    #[account(
        mut,
        constraint = user_lp_account.owner == user_stake.owner @ RewardsError::Unauthorized
    )]
    pub user_lp_account: Box<Account<'info, TokenAccount>>,

    /// Depositor's RISE token account — receives any pending rewards.
    #[account(
        mut,
        constraint = user_rise_account.mint == config.rise_mint @ RewardsError::Unauthorized,
        constraint = user_rise_account.owner == user_stake.owner @ RewardsError::Unauthorized
    )]
    pub user_rise_account: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [b"gauge_lp_vault", gauge.pool.as_ref()],
        bump,
    )]
    pub gauge_lp_vault: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [b"rewards_vault"],
        bump,
        constraint = rewards_vault.mint == config.rise_mint @ RewardsError::Unauthorized
    )]
    pub rewards_vault: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
}
