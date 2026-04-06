use anchor_lang::prelude::*;
use crate::errors::CdpError;

/// Target price scale: 1e6 (micro-USD).
/// All USD values in the CDP program use this precision.
/// e.g. $170.00 SOL → 170_000_000
const TARGET_EXPO: i32 = -6;

/// Read a USD price from a Pyth price feed account.
///
/// Returns the price scaled to 1e6 (micro-USD) as a u128.
/// Enforces a 60-second max price age — rejects stale feeds.
pub fn get_pyth_price(price_feed: &AccountInfo) -> Result<u128> {
    let feed = pyth_sdk_solana::load_price_feed_from_account_info(price_feed)
        .map_err(|_| error!(CdpError::InvalidOraclePrice))?;

    let clock = Clock::get()?;
    let price = feed
        .get_price_no_older_than(clock.unix_timestamp, 60)
        .ok_or_else(|| error!(CdpError::StaleOraclePrice))?;

    require!(price.price > 0, CdpError::InvalidOraclePrice);

    // Reject if confidence interval exceeds 2% of price
    require!(
        (price.conf as u128)
            .checked_mul(10_000)
            .ok_or(error!(CdpError::MathOverflow))?
            <= (price.price as u128)
                .checked_mul(200)
                .ok_or(error!(CdpError::MathOverflow))?,
        CdpError::InsufficientPriceConfidence
    );

    let raw = price.price as u128;
    let expo = price.expo; // e.g. -8 for most USD pairs

    // Adjust exponent to TARGET_EXPO (-6).
    // adj = expo - TARGET_EXPO; positive means multiply, negative means divide.
    let adj = expo - TARGET_EXPO;

    let scaled = if adj >= 0 {
        raw.checked_mul(10u128.pow(adj as u32))
            .ok_or_else(|| error!(CdpError::MathOverflow))?
    } else {
        raw.checked_div(10u128.pow((-adj) as u32))
            .ok_or_else(|| error!(CdpError::MathOverflow))?
    };

    require!(scaled > 0, CdpError::InvalidOraclePrice);
    Ok(scaled)
}
