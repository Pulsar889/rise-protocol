use anchor_lang::prelude::*;
use crate::state::PaymentConfig;

pub fn handler(ctx: Context<InitializePaymentConfig>, feed_id: Pubkey) -> Result<()> {
    let config = &mut ctx.accounts.payment_config;
    config.mint = ctx.accounts.mint.key();
    // Store the 32-byte Pyth feed ID as a Pubkey (pull oracle feed identifier).
    config.pyth_price_feed = feed_id;
    config.active = true;
    config.bump = ctx.bumps.payment_config;

    msg!("Payment config initialized for mint: {}", config.mint);
    msg!("Feed ID: {}", feed_id);
    msg!(
        "Native SOL: {}",
        config.is_native_sol()
    );
    Ok(())
}

#[derive(Accounts)]
pub struct InitializePaymentConfig<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = PaymentConfig::SIZE,
        seeds = [b"payment_config", mint.key().as_ref()],
        bump
    )]
    pub payment_config: Account<'info, PaymentConfig>,

    /// CHECK: The payment token mint. Pass SystemProgram ID for native SOL.
    pub mint: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}
