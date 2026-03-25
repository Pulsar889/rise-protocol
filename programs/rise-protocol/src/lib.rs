use anchor_lang::prelude::*;

declare_id!("3bCiMhJA1i1n3pRNVSgeDs5Rz5BneNWAGJMTzWRB3U5t");

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
