use asceswap_types::{U256, U512};

use crate::MathError;

pub const WAD: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);
pub const BPS_DENOMINATOR: U256 = U256::from_limbs([10_000, 0, 0, 0]);

pub fn mul_div_floor(a: U256, b: U256, denominator: U256) -> Result<U256, MathError> {
    if denominator == U256::ZERO {
        return Err(MathError::DivisionByZero);
    }

    let product: U512 = a.widening_mul(b);
    let quotient = product / U512::from(denominator);
    let limbs = quotient.as_limbs();

    if limbs[4..].iter().any(|limb| *limb != 0) {
        return Err(MathError::ArithmeticOverflow);
    }

    Ok(U256::from_limbs([limbs[0], limbs[1], limbs[2], limbs[3]]))
}
