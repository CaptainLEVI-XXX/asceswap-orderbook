use asceswap_types::{Order, Side, U256};

use crate::{mul_div_floor, MathError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedFill {
    pub claim_fill_amount: U256,
    pub collateral_amount: U256,
    pub new_filled_claim_amount: U256,
}

pub fn remaining_claim_amount(order: &Order, filled_claim_amount: U256) -> Result<U256, MathError> {
    order.validate_basic()?;
    let max_claim_amount = order.max_claim_amount();
    if filled_claim_amount > max_claim_amount {
        return Err(MathError::Overfill);
    }
    Ok(max_claim_amount - filled_claim_amount)
}

pub fn cumulative_collateral(order: &Order, filled_claim_amount: U256) -> Result<U256, MathError> {
    order.validate_basic()?;
    if filled_claim_amount > order.max_claim_amount() {
        return Err(MathError::Overfill);
    }

    match order.side {
        Side::Buy => mul_div_floor(filled_claim_amount, order.maker_amount, order.taker_amount),
        Side::Sell => mul_div_floor(filled_claim_amount, order.taker_amount, order.maker_amount),
    }
}

pub fn new_filled_claim_amount(
    order: &Order,
    filled_claim_amount: U256,
    claim_fill_amount: U256,
) -> Result<U256, MathError> {
    order.validate_basic()?;
    if claim_fill_amount == U256::ZERO {
        return Err(MathError::ZeroFill);
    }

    let new_filled = filled_claim_amount
        .checked_add(claim_fill_amount)
        .ok_or(MathError::ArithmeticOverflow)?;
    if new_filled > order.max_claim_amount() {
        return Err(MathError::Overfill);
    }

    Ok(new_filled)
}

pub fn collateral_delta_for_new_fill(
    order: &Order,
    old_filled_claim_amount: U256,
    new_filled_claim_amount: U256,
) -> Result<U256, MathError> {
    if new_filled_claim_amount <= old_filled_claim_amount {
        return Err(MathError::ZeroFill);
    }

    let old_collateral_amount = cumulative_collateral(order, old_filled_claim_amount)?;
    let new_collateral_amount = cumulative_collateral(order, new_filled_claim_amount)?;
    let collateral_amount = new_collateral_amount - old_collateral_amount;

    if collateral_amount == U256::ZERO {
        return Err(MathError::ZeroFill);
    }

    Ok(collateral_amount)
}

pub fn collateral_delta(
    order: &Order,
    old_filled_claim_amount: U256,
    claim_fill_amount: U256,
) -> Result<U256, MathError> {
    let new_filled = new_filled_claim_amount(order, old_filled_claim_amount, claim_fill_amount)?;
    collateral_delta_for_new_fill(order, old_filled_claim_amount, new_filled)
}

pub fn prepare_fill(
    order: &Order,
    filled_claim_amount: U256,
    claim_fill_amount: U256,
) -> Result<PreparedFill, MathError> {
    let new_filled_claim_amount =
        new_filled_claim_amount(order, filled_claim_amount, claim_fill_amount)?;
    let collateral_amount =
        collateral_delta_for_new_fill(order, filled_claim_amount, new_filled_claim_amount)?;

    Ok(PreparedFill {
        claim_fill_amount,
        collateral_amount,
        new_filled_claim_amount,
    })
}
