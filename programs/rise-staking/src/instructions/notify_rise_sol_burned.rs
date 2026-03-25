use anchor_lang::prelude::*;
use crate::state::GlobalPool;
use crate::errors::StakingError;

/// Called by the CDP program via CPI after burning riseSOL tokens as interest payment.
///
/// When a borrower repays CDP interest using riseSOL, those tokens are burned
/// (destroyed) — reducing the total supply. This instruction tells the staking
/// pool about that reduction so the exchange rate can adjust correctly.
///
/// The exchange rate = total_sol_staked / staking_rise_sol_supply.
/// Burning interest riseSOL without updating staking_rise_sol_supply would leave
/// the denominator too high, making the exchange rate appear lower than it is.
/// Remaining stakers would be underpaid when they eventually unstake.
///
/// Authorization: the signer must be the CDP config PDA registered on GlobalPool
/// via set_cdp_config. Only the CDP program can produce that PDA signature.
pub fn handler(ctx: Context<NotifyRiseSolBurned>, amount: u64) -> Result<()> {
    require!(amount > 0, StakingError::ZeroAmount);

    let pool = &mut ctx.accounts.global_pool;

    pool.staking_rise_sol_supply = pool
        .staking_rise_sol_supply
        .checked_sub(amount as u128)
        .ok_or(StakingError::MathOverflow)?;

    msg!("staking_rise_sol_supply reduced by {} (CDP interest burn)", amount);
    msg!("New staking_rise_sol_supply: {}", pool.staking_rise_sol_supply);

    Ok(())
}

#[derive(Accounts)]
pub struct NotifyRiseSolBurned<'info> {
    /// CDP config PDA — must match global_pool.cdp_config_pubkey.
    /// The CDP program signs this CPI with its cdp_config PDA seeds [b"cdp_config"].
    pub cdp_config: Signer<'info>,

    #[account(
        mut,
        seeds = [b"global_pool"],
        bump = global_pool.bump,
        constraint = cdp_config.key() == global_pool.cdp_config_pubkey @ StakingError::Unauthorized
    )]
    pub global_pool: Account<'info, GlobalPool>,
}
