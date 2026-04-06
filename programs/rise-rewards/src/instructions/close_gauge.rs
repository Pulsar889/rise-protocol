use anchor_lang::prelude::*;
use crate::state::Gauge;
use crate::errors::RewardsError;

/// Authority-only: closes an individual Gauge account and reclaims rent.
/// Uses raw AccountInfo to avoid deserialization failures when the struct size
/// has changed between program versions (e.g. after adding pending_emissions).
pub fn handler(ctx: Context<CloseGauge>) -> Result<()> {
    let gauge_info = &ctx.accounts.gauge;
    let authority_info = &ctx.accounts.authority;

    // Verify discriminator matches Gauge
    let data = gauge_info.try_borrow_data()?;
    require!(data[..8] == *Gauge::DISCRIMINATOR, RewardsError::Unauthorized);

    // Refuse to close if users still have LP tokens deposited — they would be unable to withdraw.
    let gauge: Gauge = AnchorDeserialize::deserialize(&mut &data[8..])?;
    require!(gauge.total_lp_deposited == 0, RewardsError::GaugeHasActiveDeposits);
    drop(data);

    // Transfer all lamports to authority
    let lamports = gauge_info.lamports();
    **gauge_info.try_borrow_mut_lamports()? -= lamports;
    **authority_info.try_borrow_mut_lamports()? += lamports;

    // Zero out account data so it can't be mistaken for a live account
    let mut data = gauge_info.try_borrow_mut_data()?;
    data.fill(0);

    msg!("Gauge closed: {}", gauge_info.key());
    Ok(())
}

#[derive(Accounts)]
pub struct CloseGauge<'info> {
    #[account(
        mut,
        constraint = authority.key() == config.authority @ RewardsError::Unauthorized
    )]
    pub authority: Signer<'info>,

    #[account(
        seeds = [b"rewards_config"],
        bump = config.bump,
    )]
    pub config: Account<'info, crate::state::RewardsConfig>,

    /// CHECK: Validated manually via discriminator check in handler.
    #[account(mut)]
    pub gauge: AccountInfo<'info>,
}
