use anchor_lang::prelude::*;
use crate::state::{CdpConfig, PaymentConfig};
use crate::errors::CdpError;

/// Authority-only: update the Pyth price feed and/or active flag on a payment config.
pub fn handler(
    ctx: Context<UpdatePaymentConfig>,
    pyth_price_feed: Option<Pubkey>,
    active: Option<bool>,
) -> Result<()> {
    let config = &mut ctx.accounts.payment_config;

    if let Some(feed) = pyth_price_feed {
        msg!("pyth_price_feed: {} → {}", config.pyth_price_feed, feed);
        config.pyth_price_feed = feed;
    }

    if let Some(is_active) = active {
        msg!("active: {} → {}", config.active, is_active);
        config.active = is_active;
    }

    msg!("Payment config updated for mint: {}", config.mint);
    Ok(())
}

#[derive(Accounts)]
pub struct UpdatePaymentConfig<'info> {
    #[account(
        constraint = authority.key() == cdp_config.authority @ CdpError::Unauthorized
    )]
    pub authority: Signer<'info>,

    #[account(
        seeds = [b"cdp_config"],
        bump = cdp_config.bump
    )]
    pub cdp_config: Account<'info, CdpConfig>,

    #[account(
        mut,
        seeds = [b"payment_config", payment_config.mint.as_ref()],
        bump = payment_config.bump
    )]
    pub payment_config: Account<'info, PaymentConfig>,
}
