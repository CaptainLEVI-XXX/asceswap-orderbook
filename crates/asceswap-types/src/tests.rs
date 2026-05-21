use crate::{Address, ClaimSide, Order, OrderError, Side, B256, U256};

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
fn validates_basic_order_shape() {
    assert_eq!(order(Side::Buy, 50, 100).validate_basic(), Ok(()));
    assert_eq!(order(Side::Sell, 100, 50).validate_basic(), Ok(()));
}

#[test]
fn rejects_zero_values() {
    let mut zero_maker = order(Side::Buy, 50, 100);
    zero_maker.maker = Address::ZERO;
    assert_eq!(zero_maker.validate_basic(), Err(OrderError::ZeroMaker));

    let mut zero_market = order(Side::Buy, 50, 100);
    zero_market.market_id = B256::ZERO;
    assert_eq!(zero_market.validate_basic(), Err(OrderError::ZeroMarket));

    let mut zero_amount = order(Side::Buy, 50, 100);
    zero_amount.maker_amount = U256::ZERO;
    assert_eq!(zero_amount.validate_basic(), Err(OrderError::ZeroAmount));
}

#[test]
fn rejects_prices_above_one() {
    assert_eq!(
        order(Side::Buy, 101, 100).validate_basic(),
        Err(OrderError::ImpossiblePrice)
    );
    assert_eq!(
        order(Side::Sell, 100, 101).validate_basic(),
        Err(OrderError::ImpossiblePrice)
    );
}

#[test]
fn exposes_claim_amount_from_order_side() {
    assert_eq!(
        order(Side::Buy, 50, 100).max_claim_amount(),
        U256::from(100)
    );
    assert_eq!(
        order(Side::Sell, 100, 50).max_claim_amount(),
        U256::from(100)
    );
}

#[test]
fn exposes_opposites() {
    assert_eq!(ClaimSide::Residual.opposite(), ClaimSide::Payoff);
    assert_eq!(ClaimSide::Payoff.opposite(), ClaimSide::Residual);
    assert_eq!(Side::Buy.opposite(), Side::Sell);
    assert_eq!(Side::Sell.opposite(), Side::Buy);
}
