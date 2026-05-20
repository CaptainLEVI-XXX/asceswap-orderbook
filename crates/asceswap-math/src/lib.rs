use asceswap_types::{Order, OrderError, Side, U256, U512};

pub const WAD: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);
pub const BPS_DENOMINATOR: U256 = U256::from_limbs([10_000, 0, 0, 0]);

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathError {
    Order(OrderError),
    DivisionByZero,
    ZeroFill,
    Overfill,
    ArithmeticOverflow,
    InvalidFeeConfig,
}

impl From<OrderError> for MathError {
    fn from(error: OrderError) -> Self {
        Self::Order(error)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreparedFill {
    pub claim_fill_amount: U256,
    pub collateral_amount: U256,
    pub new_filled_claim_amount: U256,
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use asceswap_types::{Address, ClaimSide, B256};

    fn order(side: Side, maker_amount: u64, taker_amount: u64) -> Order {
        Order {
            salt: U256::from(1),
            maker: Address::repeat_byte(1),
            market_id: B256::repeat_byte(2),
            claim: ClaimSide::Payoff,
            maker_amount: U256::from(maker_amount),
            taker_amount: U256::from(taker_amount),
            side,
            expiration: U256::ZERO,
            epoch: U256::ZERO,
            max_fee_rate_bps: 100,
        }
    }

    #[test]
    fn computes_price_wad_for_buy_and_sell_orders() {
        assert_eq!(
            price_wad(&order(Side::Buy, 50, 100)).unwrap().wad(),
            U256::from(500_000_000_000_000_000_u64)
        );
        assert_eq!(
            price_wad(&order(Side::Sell, 100, 49)).unwrap().wad(),
            U256::from(490_000_000_000_000_000_u64)
        );
    }

    #[test]
    fn computes_remaining_and_overfill() {
        let buy = order(Side::Buy, 50, 100);
        assert_eq!(
            remaining_claim_amount(&buy, U256::from(40)).unwrap(),
            U256::from(60)
        );
        assert_eq!(
            remaining_claim_amount(&buy, U256::from(101)),
            Err(MathError::Overfill)
        );
    }

    #[test]
    fn carries_rounding_dust_to_later_partial_fills() {
        let buy = order(Side::Buy, 99, 100);

        assert_eq!(
            collateral_delta(&buy, U256::ZERO, U256::from(33)).unwrap(),
            U256::from(32)
        );
        assert_eq!(
            collateral_delta(&buy, U256::from(33), U256::from(33)).unwrap(),
            U256::from(33)
        );
        assert_eq!(
            collateral_delta(&buy, U256::from(66), U256::from(34)).unwrap(),
            U256::from(34)
        );
    }

    #[test]
    fn rejects_zero_and_over_fills() {
        let buy = order(Side::Buy, 50, 100);
        assert_eq!(
            new_filled_claim_amount(&buy, U256::ZERO, U256::ZERO),
            Err(MathError::ZeroFill)
        );
        assert_eq!(
            new_filled_claim_amount(&buy, U256::from(99), U256::from(2)),
            Err(MathError::Overfill)
        );
    }

    #[test]
    fn computes_fee_from_contract_formula() {
        let price = price_wad_from_amounts(U256::from(5_000), U256::from(10_000)).unwrap();
        assert_eq!(
            taker_fee(U256::from(10_000), 100, price).unwrap(),
            U256::from(25)
        );
    }
}
