use anchor_lang::prelude::*;
use pyth_solana_receiver_sdk::price_update::PriceUpdateV2;
use crate::errors::CdpError;

/// Target price scale: 1e6 (micro-USD).
/// All USD values in the CDP program use this precision.
/// e.g. $170.00 SOL → 170_000_000
const TARGET_EXPO: i32 = -6;

/// Maximum age for a price update in seconds.
const MAX_PRICE_AGE_SECS: u64 = 60;

/// Read a USD price from a Pyth PriceUpdateV2 account (pull oracle).
///
/// `feed_id` is the 32-byte Pyth feed identifier stored in the collateral or
/// payment config. The function validates that the price update's embedded
/// feed_id matches, that the price is fresh (≤ 60 s), and that the confidence
/// interval is within 2% of the price.
///
/// Returns the price scaled to 1e6 (micro-USD) as a u128.
/// Variant for use with `remaining_accounts` — deserializes PriceUpdateV2 on demand.
pub fn get_pyth_price_info(account_info: &AccountInfo, feed_id: &[u8; 32]) -> Result<u128> {
    use anchor_lang::AccountDeserialize;
    let data = account_info.try_borrow_data()?;
    let price_update = PriceUpdateV2::try_deserialize(&mut &data[..])?;
    get_pyth_price_inner(&price_update, feed_id)
}

pub fn get_pyth_price(price_update: &Account<PriceUpdateV2>, feed_id: &[u8; 32]) -> Result<u128> {
    get_pyth_price_inner(price_update, feed_id)
}

fn get_pyth_price_inner(price_update: &PriceUpdateV2, feed_id: &[u8; 32]) -> Result<u128> {
    let clock = Clock::get()?;

    let price = price_update
        .get_price_no_older_than(&clock, MAX_PRICE_AGE_SECS, feed_id)
        .map_err(|e| {
            use pyth_solana_receiver_sdk::error::GetPriceError;
            match e {
                GetPriceError::MismatchedFeedId => error!(CdpError::WrongPriceFeed),
                _ => error!(CdpError::StaleOraclePrice),
            }
        })?;

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
    let expo = price.exponent;

    // Adjust exponent to TARGET_EXPO (-6).
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
