use anchor_lang::prelude::*;
use crate::state::GlobalPool;
use crate::errors::StakingError;

/// Authority-only: register the CDP config PDA so the staking program can
/// authorize CPI calls from rise-cdp.
///
/// Call this once after both programs are deployed. Pass in the rise-cdp
/// cdp_config PDA address (seeds = [b"cdp_config"] on the CDP program).
/// After this is set, notify_rise_sol_burned will only accept calls signed
/// by that PDA, which only rise-cdp can produce.
pub fn handler(ctx: Context<SetCdpConfig>, cdp_config_pubkey: Pubkey) -> Result<()> {
    ctx.accounts.global_pool.cdp_config_pubkey = cdp_config_pubkey;
    msg!("CDP config registered: {}", cdp_config_pubkey);
    Ok(())
}

#[derive(Accounts)]
pub struct SetCdpConfig<'info> {
    #[account(
        constraint = authority.key() == global_pool.authority @ StakingError::Unauthorized
    )]
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [b"global_pool"],
        bump = global_pool.bump
    )]
    pub global_pool: Account<'info, GlobalPool>,
}
