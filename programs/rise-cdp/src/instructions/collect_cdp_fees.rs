use anchor_lang::prelude::*;
use anchor_lang::system_program;
use crate::errors::CdpError;
use crate::state::CdpConfig;
use rise_staking::state::{GlobalPool, ProtocolTreasury};
use rise_staking::program::RiseStaking;

/// Sweep accumulated SOL from cdp_fee_vault and distribute:
///
///  90%  → pool_vault   (credited to staking pool; raises riseSOL exchange rate)
///   5%  → treasury     (protocol reserve)
///   5%  → treasury     (veRISE holder share; updates revenue_index via CPI)
///
/// Permissionless — any caller can trigger the sweep.
pub fn handler(ctx: Context<CollectCdpFees>) -> Result<()> {
    // Leave the rent-exempt minimum so the PDA is never garbage-collected.
    let rent_floor = Rent::get()?.minimum_balance(0);
    let vault_balance = ctx.accounts.cdp_fee_vault.lamports();
    if vault_balance <= rent_floor {
        msg!("CDP fee vault at or below rent floor — nothing to collect");
        return Ok(());
    }
    let total_fees = vault_balance - rent_floor;

    // ── Split: 90% staking pool, 5% reserve, 5% veRISE ─────────────────────
    const STAKING_SHARE_BPS: u64 = 9_000;
    const VERISE_SHARE_BPS: u64 = 500;

    let staking_amount = total_fees
        .checked_mul(STAKING_SHARE_BPS)
        .ok_or(CdpError::MathOverflow)?
        / 10_000;

    let remaining = total_fees
        .checked_sub(staking_amount)
        .ok_or(CdpError::MathOverflow)?;

    let verise_amount = remaining
        .checked_mul(VERISE_SHARE_BPS)
        .ok_or(CdpError::MathOverflow)?
        / (10_000 - STAKING_SHARE_BPS);

    let reserve_amount = remaining
        .checked_sub(verise_amount)
        .ok_or(CdpError::MathOverflow)?;

    let fee_vault_bump = ctx.bumps.cdp_fee_vault;
    let seeds = &[b"cdp_fee_vault".as_ref(), &[fee_vault_bump]];
    let signer = &[&seeds[..]];

    // ── Transfer staking share → pool_vault ──────────────────────────────────
    if staking_amount > 0 {
        system_program::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.cdp_fee_vault.to_account_info(),
                    to: ctx.accounts.pool_vault.to_account_info(),
                },
                signer,
            ),
            staking_amount,
        )?;

        rise_staking::cpi::credit_staking_revenue(
            CpiContext::new_with_signer(
                ctx.accounts.staking_program.to_account_info(),
                rise_staking::cpi::accounts::CreditStakingRevenue {
                    caller: ctx.accounts.cdp_fee_vault.to_account_info(),
                    global_pool: ctx.accounts.global_pool.to_account_info(),
                    pool_vault: ctx.accounts.pool_vault.to_account_info(),
                },
                signer,
            ),
            staking_amount,
        )?;

        msg!("CDP fees → staking pool: {} lamports", staking_amount);
    }

    // ── Transfer reserve share → reserve_vault ───────────────────────────────
    if reserve_amount > 0 {
        system_program::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.cdp_fee_vault.to_account_info(),
                    to: ctx.accounts.reserve_vault.to_account_info(),
                },
                signer,
            ),
            reserve_amount,
        )?;
        msg!("CDP fees → treasury reserve: {} lamports", reserve_amount);
    }

    // ── Transfer veRISE share → verise_vault ─────────────────────────────────
    if verise_amount > 0 {
        system_program::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                system_program::Transfer {
                    from: ctx.accounts.cdp_fee_vault.to_account_info(),
                    to: ctx.accounts.verise_vault.to_account_info(),
                },
                signer,
            ),
            verise_amount,
        )?;
        msg!("CDP fees → veRISE vault: {} lamports", verise_amount);
    }

    // ── CPI to staking: update revenue_index and reserve_lamports ────────────
    // Sign with cdp_config PDA — rise-staking verifies it matches global_pool.cdp_config_pubkey.
    let cdp_config_bump = ctx.accounts.cdp_config.bump;
    let cdp_config_seeds = &[b"cdp_config".as_ref(), &[cdp_config_bump]];
    let cdp_config_signer = &[&cdp_config_seeds[..]];

    rise_staking::cpi::register_external_revenue(
        CpiContext::new_with_signer(
            ctx.accounts.staking_program.to_account_info(),
            rise_staking::cpi::accounts::RegisterExternalRevenue {
                cdp_config: ctx.accounts.cdp_config.to_account_info(),
                global_pool: ctx.accounts.global_pool.to_account_info(),
                treasury: ctx.accounts.treasury.to_account_info(),
                governance_config: ctx.accounts.governance_config.to_account_info(),
            },
            cdp_config_signer,
        ),
        verise_amount,
        reserve_amount,
    )?;

    msg!(
        "CDP fee collection complete — total: {} | staking: {} | reserve: {} | veRISE: {}",
        total_fees,
        staking_amount,
        reserve_amount,
        verise_amount,
    );

    Ok(())
}

#[derive(Accounts)]
pub struct CollectCdpFees<'info> {
    /// Permissionless — any account can trigger the sweep.
    pub caller: Signer<'info>,

    /// CHECK: CDP fee vault PDA — accumulates SOL from repay_debt interest.
    #[account(
        mut,
        seeds = [b"cdp_fee_vault"],
        bump
    )]
    pub cdp_fee_vault: UncheckedAccount<'info>,

    /// CDP config PDA — signs the register_external_revenue CPI.
    #[account(
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Account<'info, CdpConfig>,

    /// ProtocolTreasury from the staking program. Written via CPI.
    #[account(
        mut,
        seeds = [b"protocol_treasury"],
        seeds::program = rise_staking::ID,
        bump = treasury.bump
    )]
    pub treasury: Account<'info, ProtocolTreasury>,

    /// CHECK: Protocol reserve vault — receives the reserve share.
    #[account(
        mut,
        seeds = [b"reserve_vault"],
        seeds::program = rise_staking::ID,
        bump
    )]
    pub reserve_vault: UncheckedAccount<'info>,

    /// CHECK: veRISE distribution vault — receives the veRISE holder share.
    #[account(
        mut,
        seeds = [b"verise_vault"],
        seeds::program = rise_staking::ID,
        bump
    )]
    pub verise_vault: UncheckedAccount<'info>,

    /// GlobalPool from staking — updated by credit_staking_revenue CPI.
    #[account(
        mut,
        seeds = [b"global_pool"],
        seeds::program = rise_staking::ID,
        bump = global_pool.bump
    )]
    pub global_pool: Box<Account<'info, GlobalPool>>,

    /// CHECK: Staking pool SOL vault — receives the 90% staking share.
    #[account(
        mut,
        seeds = [b"pool_vault"],
        seeds::program = rise_staking::ID,
        bump
    )]
    pub pool_vault: UncheckedAccount<'info>,

    /// CHECK: GovernanceConfig PDA — passed through to register_external_revenue CPI.
    /// Owner validated by the staking program inside that instruction.
    pub governance_config: UncheckedAccount<'info>,

    pub staking_program: Program<'info, RiseStaking>,
    pub system_program: Program<'info, System>,
}
