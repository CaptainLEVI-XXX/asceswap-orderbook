use asceswap_types::{Address, ClaimSide, Order, Side, B256, U256};

use crate::{
    collateral_delta, new_filled_claim_amount, price_wad, price_wad_from_amounts,
    remaining_claim_amount, taker_fee, MathError,
};

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
