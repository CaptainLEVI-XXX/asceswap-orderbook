use asceswap_types::{Order, U256};

use crate::{mul_div_floor, MathError, WAD};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Price {
    wad: U256,
}

impl Price {
    pub fn new(wad: U256) -> Self {
        Self { wad }
    }

    pub fn wad(self) -> U256 {
        self.wad
    }
}

pub fn price_wad(order: &Order) -> Result<Price, MathError> {
    order.validate_basic()?;
    let (collateral_amount, claim_amount) = order.collateral_ratio_parts();
    Ok(Price::new(mul_div_floor(
        collateral_amount,
        WAD,
        claim_amount,
    )?))
}

pub fn price_wad_from_amounts(
    actual_collateral_amount: U256,
    claim_amount: U256,
) -> Result<Price, MathError> {
    Ok(Price::new(mul_div_floor(
        actual_collateral_amount,
        WAD,
        claim_amount,
    )?))
}
