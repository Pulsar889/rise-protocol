use anchor_lang::prelude::*;
use crate::state::PaymentConfig;

pub fn handler(ctx: Context<InitializePaymentConfig>) -> Result<()> {
    let config = &mut ctx.accounts.payment_config;
    config.mint = ctx.accounts.mint.key();
    config.pyth_price_feed = ctx.accounts.pyth_price_feed.key();
    config.active = true;
    config.bump = ctx.bumps.payment_config;

    msg!("Payment config initialized for mint: {}", config.mint);
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

    /// CHECK: Pyth price feed for this payment token.
    pub pyth_price_feed: AccountInfo<'info>,

    pub system_program: Program<'info, System>,
}
