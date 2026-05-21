use asceswap_types::{Address, ClaimSide, MarketId, Order, OrderHash, Side, B256, U256};

use crate::{BookError, MarketOrderBook};

fn market() -> MarketId {
    B256::repeat_byte(9)
}

fn hash(value: u8) -> OrderHash {
    B256::repeat_byte(value)
}

fn order(claim: ClaimSide, side: Side, maker_amount: u64, taker_amount: u64) -> Order {
    Order {
        salt: U256::from(1),
        maker: Address::repeat_byte(1),
        market_id: market(),
        claim,
        maker_amount: U256::from(maker_amount),
        taker_amount: U256::from(taker_amount),
        side,
        expiration: U256::ZERO,
        epoch: U256::ZERO,
        max_fee_rate_bps: 100,
    }
}

#[test]
fn keeps_price_time_priority_for_bids_and_asks() {
    let mut book = MarketOrderBook::new(market());
    book.insert(hash(1), order(ClaimSide::Payoff, Side::Sell, 100, 50))
        .unwrap();
    book.insert(hash(2), order(ClaimSide::Payoff, Side::Sell, 100, 49))
        .unwrap();
    book.insert(hash(3), order(ClaimSide::Payoff, Side::Sell, 100, 49))
        .unwrap();
    book.insert(hash(4), order(ClaimSide::Payoff, Side::Buy, 51, 100))
        .unwrap();
    book.insert(hash(5), order(ClaimSide::Payoff, Side::Buy, 50, 100))
        .unwrap();

    let asks: Vec<_> = book
        .iter_priority(ClaimSide::Payoff, Side::Sell)
        .into_iter()
        .map(|order| order.hash)
        .collect();
    assert_eq!(asks, vec![hash(2), hash(3), hash(1)]);

    let bids: Vec<_> = book
        .iter_priority(ClaimSide::Payoff, Side::Buy)
        .into_iter()
        .map(|order| order.hash)
        .collect();
    assert_eq!(bids, vec![hash(4), hash(5)]);
}

#[test]
fn aggregates_depth_at_price() {
    let mut book = MarketOrderBook::new(market());
    book.insert(hash(1), order(ClaimSide::Payoff, Side::Sell, 100, 49))
        .unwrap();
    book.insert(hash(2), order(ClaimSide::Payoff, Side::Sell, 200, 98))
        .unwrap();

    let depth = book.depth(ClaimSide::Payoff, Side::Sell).unwrap();
    assert_eq!(depth.len(), 1);
    assert_eq!(depth[0].total_claim_amount, U256::from(300));
    assert_eq!(depth[0].order_count, 2);
}

#[test]
fn removes_fully_filled_order() {
    let mut book = MarketOrderBook::new(market());
    book.insert(hash(1), order(ClaimSide::Payoff, Side::Sell, 100, 49))
        .unwrap();

    assert_eq!(
        book.apply_fill(hash(1), U256::from(100)).unwrap(),
        U256::from(100)
    );
    assert!(!book.contains(hash(1)));
    assert!(book.best(ClaimSide::Payoff, Side::Sell).is_none());
}

#[test]
fn rejects_wrong_market_and_duplicates() {
    let mut book = MarketOrderBook::new(market());
    let mut wrong_market = order(ClaimSide::Payoff, Side::Sell, 100, 49);
    wrong_market.market_id = B256::repeat_byte(8);

    assert!(matches!(
        book.insert(hash(1), wrong_market),
        Err(BookError::WrongMarket { .. })
    ));

    let sell = order(ClaimSide::Payoff, Side::Sell, 100, 49);
    book.insert(hash(1), sell.clone()).unwrap();
    assert_eq!(
        book.insert(hash(1), sell),
        Err(BookError::DuplicateOrder(hash(1)))
    );
}
