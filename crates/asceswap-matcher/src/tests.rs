use asceswap_orderbook::MarketOrderBook;
use asceswap_types::{Address, ClaimSide, MarketId, MatchKind, Order, OrderHash, Side, B256, U256};

use crate::{
    plan_direct, plan_match, plan_merge_assisted, plan_mint_assisted, MatchConfig, MatchError,
    CONTRACT_MAX_MAKER_ORDERS,
};

fn market() -> MarketId {
    B256::repeat_byte(9)
}

fn hash(value: u8) -> OrderHash {
    B256::repeat_byte(value)
}

fn order(claim: ClaimSide, side: Side, maker_amount: u64, taker_amount: u64, maker: u8) -> Order {
    Order {
        salt: U256::from(maker),
        maker: Address::repeat_byte(maker),
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
fn plans_direct_buy_with_price_improvement() {
    let mut book = MarketOrderBook::new(market());
    book.insert(hash(1), order(ClaimSide::Payoff, Side::Sell, 100, 49, 1))
        .unwrap();
    let taker = order(ClaimSide::Payoff, Side::Buy, 50, 100, 2);

    let plan = plan_direct(&book, &taker, U256::ZERO, MatchConfig::default())
        .unwrap()
        .unwrap();

    assert_eq!(plan.match_kind, MatchKind::Direct);
    assert_eq!(plan.taker_claim_fill_amount, U256::from(100));
    assert_eq!(plan.taker_collateral_amount, U256::from(50));
    assert_eq!(plan.total_maker_collateral_amount, U256::from(49));
    assert_eq!(plan.taker_actual_collateral_amount, U256::from(49));
    assert_eq!(plan.maker_fills[0].order_hash, hash(1));
}

#[test]
fn plans_direct_sell_with_price_improvement() {
    let mut book = MarketOrderBook::new(market());
    book.insert(hash(1), order(ClaimSide::Payoff, Side::Buy, 50, 100, 1))
        .unwrap();
    let taker = order(ClaimSide::Payoff, Side::Sell, 100, 48, 2);

    let plan = plan_direct(&book, &taker, U256::ZERO, MatchConfig::default())
        .unwrap()
        .unwrap();

    assert_eq!(plan.match_kind, MatchKind::Direct);
    assert_eq!(plan.taker_claim_fill_amount, U256::from(100));
    assert_eq!(plan.taker_collateral_amount, U256::from(48));
    assert_eq!(plan.total_maker_collateral_amount, U256::from(50));
    assert_eq!(plan.taker_actual_collateral_amount, U256::from(50));
}

#[test]
fn plans_mint_assisted_buy_buy() {
    let mut book = MarketOrderBook::new(market());
    book.insert(hash(1), order(ClaimSide::Residual, Side::Buy, 45, 100, 1))
        .unwrap();
    let taker = order(ClaimSide::Payoff, Side::Buy, 60, 100, 2);

    let plan = plan_mint_assisted(&book, &taker, U256::ZERO, MatchConfig::default())
        .unwrap()
        .unwrap();

    assert_eq!(plan.match_kind, MatchKind::MintAssisted);
    assert_eq!(plan.taker_claim_fill_amount, U256::from(100));
    assert_eq!(plan.taker_collateral_amount, U256::from(60));
    assert_eq!(plan.total_maker_collateral_amount, U256::from(45));
    assert_eq!(plan.taker_actual_collateral_amount, U256::from(55));
}

#[test]
fn plans_merge_assisted_sell_sell() {
    let mut book = MarketOrderBook::new(market());
    book.insert(hash(1), order(ClaimSide::Residual, Side::Sell, 100, 35, 1))
        .unwrap();
    let taker = order(ClaimSide::Payoff, Side::Sell, 100, 60, 2);

    let plan = plan_merge_assisted(&book, &taker, U256::ZERO, MatchConfig::default())
        .unwrap()
        .unwrap();

    assert_eq!(plan.match_kind, MatchKind::MergeAssisted);
    assert_eq!(plan.taker_claim_fill_amount, U256::from(100));
    assert_eq!(plan.taker_collateral_amount, U256::from(60));
    assert_eq!(plan.total_maker_collateral_amount, U256::from(35));
    assert_eq!(plan.taker_actual_collateral_amount, U256::from(65));
}

#[test]
fn returns_none_when_prices_do_not_cross() {
    let mut book = MarketOrderBook::new(market());
    book.insert(hash(1), order(ClaimSide::Payoff, Side::Sell, 100, 51, 1))
        .unwrap();
    let taker = order(ClaimSide::Payoff, Side::Buy, 50, 100, 2);

    assert_eq!(
        plan_direct(&book, &taker, U256::ZERO, MatchConfig::default()).unwrap(),
        None
    );
}

#[test]
fn respects_contract_maker_limit() {
    let mut book = MarketOrderBook::new(market());
    for index in 1..=40_u8 {
        book.insert(
            hash(index),
            order(ClaimSide::Payoff, Side::Sell, 1, 1, index),
        )
        .unwrap();
    }
    let taker = order(ClaimSide::Payoff, Side::Buy, 40, 40, 200);

    let plan = plan_direct(&book, &taker, U256::ZERO, MatchConfig::default())
        .unwrap()
        .unwrap();

    assert_eq!(plan.maker_fills.len(), CONTRACT_MAX_MAKER_ORDERS);
    assert_eq!(plan.taker_claim_fill_amount, U256::from(32));
}

#[test]
fn rejects_invalid_config_above_contract_limit() {
    let book = MarketOrderBook::new(market());
    let taker = order(ClaimSide::Payoff, Side::Buy, 50, 100, 2);

    assert_eq!(
        plan_match(
            &book,
            &taker,
            U256::ZERO,
            MatchConfig {
                max_maker_orders: CONTRACT_MAX_MAKER_ORDERS + 1
            },
        ),
        Err(MatchError::InvalidConfig)
    );
}
