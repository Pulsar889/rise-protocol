use anchor_lang::prelude::*;

declare_id!("3bCiMhJA1i1n3pRNVSgeDs5Rz5BneNWAGJMTzWRB3U5t");

/// Placeholder router program reserved for future cross-program routing instructions.
/// This program should not be called directly. It exists as an anchor point for
/// composability features (e.g. atomic multi-step operations across rise-staking,
/// rise-cdp, and rise-governance) that will be added in a future release.
#[program]
pub mod rise_protocol {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        msg!("Greetings from: {:?}", ctx.program_id);
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize {}
