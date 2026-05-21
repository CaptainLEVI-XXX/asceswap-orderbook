use asceswap_types::U256;

use crate::{mul_div_floor, MathError, Price, BPS_DENOMINATOR, WAD};

pub fn taker_fee(
    claim_amount_filled: U256,
    fee_rate_bps: u16,
    price_wad: Price,
) -> Result<U256, MathError> {
    if price_wad.wad() > WAD {
        return Err(MathError::InvalidFeeConfig);
    }

    let variance_wad = mul_div_floor(price_wad.wad(), WAD - price_wad.wad(), WAD)?;
    let variance_adjusted_notional = mul_div_floor(claim_amount_filled, variance_wad, WAD)?;
    mul_div_floor(
        variance_adjusted_notional,
        U256::from(fee_rate_bps),
        BPS_DENOMINATOR,
    )
}
