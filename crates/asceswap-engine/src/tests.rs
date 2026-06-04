use asceswap_matcher::MatchConfig;
use asceswap_state::{OrderState, ReservationStatus};
use asceswap_types::{Address, ClaimSide, Order, OrderError, Side, B256, U256};
use asceswap_validation::{order_hash, OrderValidationContext, SignatureCheck, ValidationError};

use crate::{
    AsceSwapEngine, CancelOrder, EngineError, EngineEvent, SubmitOrder, SubmitOrderOutcome,
};

fn market_id() -> B256 {
    B256::repeat_byte(7)
}

fn maker_address(value: u8) -> Address {
    Address::repeat_byte(value)
}

fn sell_order(salt: u64, maker: u8, claim_amount: u64, collateral_amount: u64) -> Order {
    Order {
        salt: U256::from(salt),
        maker: maker_address(maker),
        market_id: market_id(),
        claim: ClaimSide::Payoff,
        maker_amount: U256::from(claim_amount),
        taker_amount: U256::from(collateral_amount),
        side: Side::Sell,
        expiration: U256::ZERO,
        epoch: U256::from(1),
        max_fee_rate_bps: 100,
    }
}

fn buy_order(salt: u64, maker: u8, claim_amount: u64, collateral_amount: u64) -> Order {
    Order {
        salt: U256::from(salt),
        maker: maker_address(maker),
        market_id: market_id(),
        claim: ClaimSide::Payoff,
        maker_amount: U256::from(collateral_amount),
        taker_amount: U256::from(claim_amount),
        side: Side::Buy,
        expiration: U256::ZERO,
        epoch: U256::from(1),
        max_fee_rate_bps: 100,
    }
}

fn validation(order: &Order, now: u64) -> OrderValidationContext {
    OrderValidationContext::new(now)
        .with_expected_order_hash(order_hash(order))
        .with_maker_epoch(order.epoch)
        .with_fee_rate_bps(0)
        .with_signature(SignatureCheck::Valid)
        .with_required_signature(true)
}

fn submit(order: Order, now: u64) -> SubmitOrder {
    SubmitOrder::new(order.clone(), validation(&order, now))
}

fn signed_submit(order: Order, now: u64, signature_byte: u8) -> SubmitOrder {
    submit(order, now).with_signature(Some(vec![signature_byte; 65]))
}

fn rest_maker(engine: &mut AsceSwapEngine) -> Order {
    let maker_order = sell_order(1, 1, 100, 40);
    let result = engine
        .submit_order(submit(maker_order.clone(), 100))
        .unwrap();
    assert!(matches!(result.outcome, SubmitOrderOutcome::Rested { .. }));
    maker_order
}

#[test]
fn accepts_valid_order_and_rests_it_on_book() {
    let mut engine = AsceSwapEngine::default();
    let order = sell_order(1, 1, 100, 40);
    let order_hash = order_hash(&order);

    let result = engine.submit_order(submit(order.clone(), 100)).unwrap();

    assert!(matches!(result.outcome, SubmitOrderOutcome::Rested { .. }));
    assert_eq!(
        engine.order_record(order_hash).unwrap().state(),
        OrderState::Open
    );
    assert!(engine
        .market_book(market_id())
        .unwrap()
        .contains(order_hash));
    assert_eq!(
        result.events.last(),
        Some(&EngineEvent::OrderOpened { order_hash })
    );
}

#[test]
fn rejects_invalid_order_with_recorded_reason() {
    let mut engine = AsceSwapEngine::default();
    let mut order = sell_order(1, 1, 100, 40);
    order.maker = Address::ZERO;
    let order_hash = order_hash(&order);

    let result = engine.submit_order(submit(order, 100)).unwrap();

    assert_eq!(
        result.outcome,
        SubmitOrderOutcome::Rejected {
            reason: ValidationError::BasicOrder(OrderError::ZeroMaker)
        }
    );
    assert_eq!(
        engine.order_record(order_hash).unwrap().state(),
        OrderState::Rejected
    );
}

#[test]
fn crossed_order_creates_reservation_and_marks_orders_reserved() {
    let mut engine = AsceSwapEngine::default();
    let maker_order = rest_maker(&mut engine);
    let taker_order = buy_order(2, 2, 100, 50);
    let maker_hash = order_hash(&maker_order);
    let taker_hash = order_hash(&taker_order);

    let result = engine
        .submit_order(submit(taker_order, 101).with_reservation_ttl_secs(Some(10)))
        .unwrap();

    let reservation_id = match result.outcome {
        SubmitOrderOutcome::Matched {
            reservation_id,
            plan,
            ..
        } => {
            assert_eq!(plan.maker_fills.len(), 1);
            reservation_id
        }
        other => panic!("expected match, got {other:?}"),
    };

    assert_eq!(
        engine.order_record(taker_hash).unwrap().state(),
        OrderState::Reserved
    );
    assert_eq!(
        engine.order_record(maker_hash).unwrap().state(),
        OrderState::Reserved
    );
    assert_eq!(
        engine.reservation(reservation_id).unwrap().status,
        ReservationStatus::Reserved
    );
}

#[test]
fn reserved_maker_is_skipped_for_next_taker() {
    let mut engine = AsceSwapEngine::default();
    let first_maker = sell_order(1, 1, 100, 40);
    let second_maker = sell_order(2, 2, 100, 45);
    let first_maker_hash = order_hash(&first_maker);
    let second_maker_hash = order_hash(&second_maker);
    engine.submit_order(submit(first_maker, 100)).unwrap();
    engine.submit_order(submit(second_maker, 101)).unwrap();

    let first_taker = buy_order(3, 3, 100, 50);
    let first_result = engine
        .submit_order(submit(first_taker, 102).with_reservation_ttl_secs(Some(10)))
        .unwrap();
    match first_result.outcome {
        SubmitOrderOutcome::Matched { plan, .. } => {
            assert_eq!(plan.maker_fills[0].order_hash, first_maker_hash);
        }
        other => panic!("expected first match, got {other:?}"),
    }

    let second_taker = buy_order(4, 4, 100, 50);
    let second_result = engine
        .submit_order(submit(second_taker, 103).with_reservation_ttl_secs(Some(10)))
        .unwrap();

    match second_result.outcome {
        SubmitOrderOutcome::Matched { plan, .. } => {
            assert_eq!(plan.maker_fills.len(), 1);
            assert_eq!(plan.maker_fills[0].order_hash, second_maker_hash);
        }
        other => panic!("expected second match, got {other:?}"),
    }
    assert_eq!(
        engine.order_record(first_maker_hash).unwrap().state(),
        OrderState::Reserved
    );
    assert_eq!(
        engine.order_record(second_maker_hash).unwrap().state(),
        OrderState::Reserved
    );
}

#[test]
fn matched_order_exposes_contract_settlement_payload() {
    let mut engine = AsceSwapEngine::default();
    let maker_order = sell_order(1, 1, 100, 40);
    let taker_order = buy_order(2, 2, 100, 50);
    let maker_hash = order_hash(&maker_order);
    engine
        .submit_order(signed_submit(maker_order.clone(), 100, 1))
        .unwrap();

    let result = engine
        .submit_order(signed_submit(taker_order.clone(), 101, 2))
        .unwrap();

    let (reservation_id, settlement) = match result.outcome {
        SubmitOrderOutcome::Matched {
            reservation_id,
            settlement: Some(settlement),
            ..
        } => (reservation_id, settlement),
        other => panic!("expected settlement payload, got {other:?}"),
    };
    assert_eq!(settlement.taker_order, taker_order);
    assert_eq!(settlement.taker_signature, vec![2; 65]);
    assert_eq!(settlement.maker_orders, vec![maker_order]);
    assert_eq!(settlement.maker_signatures, vec![vec![1; 65]]);
    assert_eq!(settlement.taker_claim_fill_amount, U256::from(100));
    assert_eq!(settlement.maker_claim_fill_amounts, vec![U256::from(100)]);

    let fetched = engine.settlement_payload(reservation_id).unwrap();
    assert_eq!(fetched, settlement);
    assert_eq!(
        engine.order_record(maker_hash).unwrap().signature,
        Some(vec![1; 65])
    );
}

#[test]
fn submitted_reservation_commit_applies_fills_and_removes_filled_maker() {
    let mut engine = AsceSwapEngine::default();
    let maker_order = rest_maker(&mut engine);
    let taker_order = buy_order(2, 2, 100, 50);
    let maker_hash = order_hash(&maker_order);
    let taker_hash = order_hash(&taker_order);
    let result = engine.submit_order(submit(taker_order, 101)).unwrap();
    let reservation_id = match result.outcome {
        SubmitOrderOutcome::Matched { reservation_id, .. } => reservation_id,
        other => panic!("expected match, got {other:?}"),
    };

    engine
        .mark_reservation_submitted(reservation_id, 102)
        .unwrap();
    let commit = engine.commit_reservation(reservation_id).unwrap();

    assert_eq!(
        engine.reservation(reservation_id).unwrap().status,
        ReservationStatus::Committed
    );
    assert_eq!(
        engine.order_record(taker_hash).unwrap().state(),
        OrderState::Filled
    );
    assert_eq!(
        engine.order_record(maker_hash).unwrap().state(),
        OrderState::Filled
    );
    assert!(!engine
        .market_book(market_id())
        .unwrap()
        .contains(maker_hash));
    assert!(commit
        .events
        .iter()
        .any(|event| matches!(event, EngineEvent::ReservationCommitted { .. })));
}

#[test]
fn reservation_expiry_restores_maker_and_inactivates_non_resting_taker() {
    let mut engine = AsceSwapEngine::default();
    let maker_order = rest_maker(&mut engine);
    let taker_order = buy_order(2, 2, 100, 50);
    let maker_hash = order_hash(&maker_order);
    let taker_hash = order_hash(&taker_order);
    let result = engine
        .submit_order(submit(taker_order, 100).with_reservation_ttl_secs(Some(10)))
        .unwrap();
    let reservation_id = match result.outcome {
        SubmitOrderOutcome::Matched { reservation_id, .. } => reservation_id,
        other => panic!("expected match, got {other:?}"),
    };

    engine.expire_reservation(reservation_id, 110).unwrap();

    assert_eq!(
        engine.reservation(reservation_id).unwrap().status,
        ReservationStatus::Expired
    );
    assert_eq!(
        engine.order_record(maker_hash).unwrap().state(),
        OrderState::Open
    );
    assert_eq!(
        engine.order_record(taker_hash).unwrap().state(),
        OrderState::Inactive
    );
    assert!(engine
        .market_book(market_id())
        .unwrap()
        .contains(maker_hash));
}

#[test]
fn snapshot_recovery_preserves_books_records_and_reservations() {
    let mut engine = AsceSwapEngine::default();
    let maker_order = rest_maker(&mut engine);
    let taker_order = buy_order(2, 2, 100, 50);
    let maker_hash = order_hash(&maker_order);
    let taker_hash = order_hash(&taker_order);
    let result = engine
        .submit_order(submit(taker_order, 100).with_reservation_ttl_secs(Some(10)))
        .unwrap();
    let reservation_id = match result.outcome {
        SubmitOrderOutcome::Matched { reservation_id, .. } => reservation_id,
        other => panic!("expected match, got {other:?}"),
    };

    let snapshot = engine.snapshot();
    let mut recovered = AsceSwapEngine::from_snapshot(MatchConfig::default(), snapshot).unwrap();

    assert_eq!(
        recovered.order_record(maker_hash).unwrap().state(),
        OrderState::Reserved
    );
    assert_eq!(
        recovered.order_record(taker_hash).unwrap().state(),
        OrderState::Reserved
    );
    assert_eq!(
        recovered.reservation(reservation_id).unwrap().status,
        ReservationStatus::Reserved
    );

    recovered.expire_reservation(reservation_id, 110).unwrap();
    assert_eq!(
        recovered.order_record(maker_hash).unwrap().state(),
        OrderState::Open
    );
    assert_eq!(
        recovered.order_record(taker_hash).unwrap().state(),
        OrderState::Inactive
    );
}

#[test]
fn snapshot_recovery_preserves_same_price_fifo_priority() {
    let mut engine = AsceSwapEngine::default();
    let first = sell_order(1, 1, 100, 40);
    let second = sell_order(2, 2, 100, 40);
    let first_hash = order_hash(&first);
    let second_hash = order_hash(&second);
    engine.submit_order(submit(first, 100)).unwrap();
    engine.submit_order(submit(second, 101)).unwrap();

    let mut snapshot = engine.snapshot();
    snapshot.orders.reverse();
    let recovered = AsceSwapEngine::from_snapshot(MatchConfig::default(), snapshot).unwrap();
    let priority = recovered
        .market_book(market_id())
        .unwrap()
        .iter_priority(ClaimSide::Payoff, Side::Sell)
        .into_iter()
        .map(|order| order.hash)
        .collect::<Vec<_>>();

    assert_eq!(priority, vec![first_hash, second_hash]);
}

#[test]
fn snapshot_recovery_rejects_order_hash_mismatch() {
    let mut engine = AsceSwapEngine::default();
    let order = rest_maker(&mut engine);
    let actual_hash = order_hash(&order);
    let mut snapshot = engine.snapshot();
    let wrong_hash = B256::repeat_byte(99);
    snapshot.orders[0].hash = wrong_hash;

    assert_eq!(
        AsceSwapEngine::from_snapshot(MatchConfig::default(), snapshot).unwrap_err(),
        EngineError::SnapshotOrderHashMismatch {
            expected: wrong_hash,
            actual: actual_hash,
        }
    );
}

#[test]
fn snapshot_recovery_rejects_over_reserved_orders() {
    let mut engine = AsceSwapEngine::default();
    rest_maker(&mut engine);
    let taker_order = buy_order(2, 2, 100, 50);
    let result = engine
        .submit_order(submit(taker_order, 100).with_reservation_ttl_secs(Some(10)))
        .unwrap();
    assert!(matches!(result.outcome, SubmitOrderOutcome::Matched { .. }));

    let mut snapshot = engine.snapshot();
    let order_hash = snapshot.reservations[0].legs[0].order_hash;
    snapshot.reservations[0].legs[0].claim_amount = U256::from(101);

    assert_eq!(
        AsceSwapEngine::from_snapshot(MatchConfig::default(), snapshot).unwrap_err(),
        EngineError::ReservedAmountExceedsAvailable {
            order_hash,
            reserved: U256::from(101),
            available: U256::from(100),
        }
    );
}

#[test]
fn cancel_removes_resting_order_from_book() {
    let mut engine = AsceSwapEngine::default();
    let order = rest_maker(&mut engine);
    let order_hash = order_hash(&order);

    let result = engine.cancel_order(CancelOrder::new(order_hash)).unwrap();

    assert_eq!(result.order_hash, order_hash);
    assert_eq!(
        engine.order_record(order_hash).unwrap().state(),
        OrderState::Cancelled
    );
    assert!(!engine
        .market_book(market_id())
        .unwrap()
        .contains(order_hash));
}

#[test]
fn valid_no_match_order_can_be_left_inactive() {
    let mut engine = AsceSwapEngine::default();
    let order = buy_order(1, 1, 100, 40);
    let order_hash = order_hash(&order);

    let result = engine
        .submit_order(submit(order, 100).with_rest_on_no_match(false))
        .unwrap();

    assert_eq!(result.outcome, SubmitOrderOutcome::Inactive);
    assert_eq!(
        engine.order_record(order_hash).unwrap().state(),
        OrderState::Inactive
    );
    assert_eq!(engine.market_book(market_id()).unwrap().order_count(), 0);
}

#[test]
fn duplicate_order_submission_is_rejected_by_engine() {
    let mut engine = AsceSwapEngine::default();
    let order = sell_order(1, 1, 100, 40);
    let order_hash = order_hash(&order);

    engine.submit_order(submit(order.clone(), 100)).unwrap();
    assert_eq!(
        engine.submit_order(submit(order, 101)),
        Err(EngineError::DuplicateOrder(order_hash))
    );
}
