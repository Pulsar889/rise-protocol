use anchor_lang::prelude::*;
use crate::state::ProtocolTreasury;
use crate::errors::StakingError;

pub fn handler(
    ctx: Context<UpdateTreasuryConfig>,
    team_wallet: Option<Pubkey>,
    team_fee_bps: Option<u16>,
    verise_share_bps: Option<u16>,
) -> Result<()> {
    let treasury = &mut ctx.accounts.treasury;

    if let Some(wallet) = team_wallet {
        treasury.team_wallet = wallet;
        msg!("Team wallet updated to: {}", wallet);
    }

    if let Some(fee) = team_fee_bps {
        require!(fee <= 5_000, StakingError::InvalidFeeBps); // max 50%
        treasury.team_fee_bps = fee;
        msg!("Team fee updated to: {} bps", fee);
    }

    if let Some(share) = verise_share_bps {
        require!(share <= 10_000, StakingError::InvalidFeeBps);
        treasury.verise_share_bps = share;
        msg!("veRISE share updated to: {} bps", share);
    }

    Ok(())
}

#[derive(Accounts)]
pub struct UpdateTreasuryConfig<'info> {
    /// Only authority can update treasury config.
    #[account(
        constraint = authority.key() == treasury.authority
    )]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"protocol_treasury"],
        bump = treasury.bump
    )]
    pub treasury: Account<'info, ProtocolTreasury>,
}
