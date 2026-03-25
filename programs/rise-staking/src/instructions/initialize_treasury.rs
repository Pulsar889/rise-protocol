use anchor_lang::prelude::*;
use crate::state::ProtocolTreasury;
use crate::errors::StakingError;

pub fn handler(
    ctx: Context<InitializeTreasury>,
    team_wallet: Pubkey,
    team_fee_bps: u16,
    verise_share_bps: u16,
) -> Result<()> {
    require!(team_fee_bps <= 5_000, StakingError::InvalidFeeBps); // max 50%
    require!(verise_share_bps <= 10_000, StakingError::InvalidFeeBps);

    let treasury = &mut ctx.accounts.treasury;

    treasury.authority = ctx.accounts.authority.key();
    treasury.team_wallet = team_wallet;
    treasury.team_fee_bps = team_fee_bps;
    treasury.verise_share_bps = verise_share_bps;
    treasury.reserve_lamports = 0;
    treasury.revenue_index = 0;
    treasury.total_distributed = 0;
    treasury.last_collection_epoch = Clock::get()?.epoch;
    treasury.bump = ctx.bumps.treasury;

    msg!("Treasury initialized");
    msg!("Team wallet: {}", team_wallet);
    msg!("Team fee: {} bps", team_fee_bps);
    msg!("veRISE share: {} bps", verise_share_bps);

    Ok(())
}

#[derive(Accounts)]
pub struct InitializeTreasury<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = ProtocolTreasury::SIZE,
        seeds = [b"protocol_treasury"],
        bump
    )]
    pub treasury: Account<'info, ProtocolTreasury>,

    pub system_program: Program<'info, System>,
}
