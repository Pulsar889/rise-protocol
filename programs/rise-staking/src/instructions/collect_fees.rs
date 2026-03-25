use anchor_lang::prelude::*;
use anchor_lang::system_program;
use crate::state::{GlobalPool, ProtocolTreasury};
use crate::errors::StakingError;

pub fn handler(ctx: Context<CollectFees>) -> Result<()> {
    let current_epoch = Clock::get()?.epoch;

    require!(
        current_epoch > ctx.accounts.treasury.last_collection_epoch,
        StakingError::InvalidFeeBps
    );

    let pool = &ctx.accounts.pool;
    let vault_balance = ctx.accounts.pool_vault.lamports() as u128;

    // Sweepable fees = vault excess above liquid buffer and pending withdrawals.
    // Vault invariant: vault = liquid_buffer + pending_withdrawals + fee_excess
    let sweepable = vault_balance
        .checked_sub(pool.liquid_buffer_lamports)
        .ok_or(StakingError::MathOverflow)?
        .checked_sub(pool.pending_withdrawals_lamports)
        .ok_or(StakingError::MathOverflow)?;

    if sweepable == 0 {
        msg!("No fees to collect this epoch");
        ctx.accounts.treasury.last_collection_epoch = current_epoch;
        return Ok(());
    }

    let total_fees = sweepable as u64;

    if total_fees == 0 {
        msg!("No fees to collect this epoch");
        return Ok(());
    }

    let treasury = &mut ctx.accounts.treasury;

    // Team cut (25% of total)
    let team_amount = treasury
        .team_cut(total_fees)
        .ok_or(StakingError::MathOverflow)?;

    // veRISE holder share (25% of total)
    let verise_amount = treasury
        .verise_cut(total_fees)
        .ok_or(StakingError::MathOverflow)?;

    // Treasury reserve (50% of total — remainder after team + veRISE)
    let reserve_amount = total_fees
        .checked_sub(team_amount)
        .ok_or(StakingError::MathOverflow)?
        .checked_sub(verise_amount)
        .ok_or(StakingError::MathOverflow)?;

    let vault_bump = ctx.bumps.pool_vault;
    let seeds = &[b"pool_vault".as_ref(), &[vault_bump]];
    let signer = &[&seeds[..]];

    // Transfer team cut
    if team_amount > 0 {
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.pool_vault.to_account_info(),
                to: ctx.accounts.team_wallet.to_account_info(),
            },
            signer,
        );
        system_program::transfer(cpi_ctx, team_amount)?;
        msg!("Team cut sent: {} lamports", team_amount);
    }

    // Transfer reserve to treasury vault
    if reserve_amount > 0 {
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.pool_vault.to_account_info(),
                to: ctx.accounts.treasury_vault.to_account_info(),
            },
            signer,
        );
        system_program::transfer(cpi_ctx, reserve_amount)?;

        treasury.reserve_lamports = treasury
            .reserve_lamports
            .checked_add(reserve_amount as u128)
            .ok_or(StakingError::MathOverflow)?;

        msg!("Treasury reserve received: {} lamports", reserve_amount);
    }

    // Transfer veRISE share to treasury vault and update revenue index
    if verise_amount > 0 {
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.pool_vault.to_account_info(),
                to: ctx.accounts.treasury_vault.to_account_info(),
            },
            signer,
        );
        system_program::transfer(cpi_ctx, verise_amount)?;

        // Update revenue index so veRISE holders can claim
        // index += verise_amount * INDEX_SCALE
        // (divided by total_verise when claiming — stored as raw amount here
        // and divided per-user in claim_revenue_share)
        treasury.revenue_index = treasury
            .revenue_index
            .checked_add(
                (verise_amount as u128)
                    .checked_mul(ProtocolTreasury::INDEX_SCALE)
                    .ok_or(StakingError::MathOverflow)?
            )
            .ok_or(StakingError::MathOverflow)?;

        treasury.total_distributed = treasury
            .total_distributed
            .checked_add(verise_amount as u128)
            .ok_or(StakingError::MathOverflow)?;

        msg!("veRISE revenue index updated: {}", treasury.revenue_index);
        msg!("veRISE rewards queued: {} lamports", verise_amount);
    }

    treasury.last_collection_epoch = current_epoch;

    msg!("Fee collection complete");
    msg!("Total fees: {} | Team: {} | Reserve: {} | veRISE: {}",
        total_fees, team_amount, reserve_amount, verise_amount);

    Ok(())
}

#[derive(Accounts)]
pub struct CollectFees<'info> {
    pub caller: Signer<'info>,

    #[account(
        seeds = [b"global_pool"],
        bump = pool.bump
    )]
    pub pool: Account<'info, GlobalPool>,

    #[account(
        mut,
        seeds = [b"protocol_treasury"],
        bump = treasury.bump
    )]
    pub treasury: Account<'info, ProtocolTreasury>,

    /// CHECK: Pool SOL vault.
    #[account(
        mut,
        seeds = [b"pool_vault"],
        bump
    )]
    pub pool_vault: UncheckedAccount<'info>,

    /// CHECK: Treasury SOL vault.
    #[account(
        mut,
        seeds = [b"treasury_vault"],
        bump
    )]
    pub treasury_vault: UncheckedAccount<'info>,

    /// CHECK: Team salary wallet.
    #[account(
        mut,
        constraint = team_wallet.key() == treasury.team_wallet
    )]
    pub team_wallet: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}
