use asceswap_engine::{AsceSwapEngine, EngineEvent, SubmitOrder, SubmitOrderOutcome};
use asceswap_matcher::MatchConfig;
use asceswap_state::{OrderState, ReservationStatus};
use asceswap_types::{Address, ClaimSide, Order, Side, B256, U256};
use asceswap_validation::{order_hash, OrderValidationContext, SignatureCheck};

use crate::{EngineStore, InMemoryEngineStore, StorageError, StoredEngineEvent, StoredFill};

fn market_id() -> B256 {
    B256::repeat_byte(8)
}

fn sell_order(salt: u64, maker: u8, claim_amount: u64, collateral_amount: u64) -> Order {
    Order {
        salt: U256::from(salt),
        maker: Address::repeat_byte(maker),
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
        maker: Address::repeat_byte(maker),
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
        .with_signature(SignatureCheck::Valid)
        .with_required_signature(true)
}

fn submit(order: Order, now: u64) -> SubmitOrder {
    SubmitOrder::new(order.clone(), validation(&order, now))
}

fn signed_submit(order: Order, now: u64, signature_byte: u8) -> SubmitOrder {
    submit(order, now).with_signature(Some(vec![signature_byte; 65]))
}

fn matched_engine() -> (AsceSwapEngine, B256, B256, B256) {
    let mut engine = AsceSwapEngine::default();
    let maker_order = sell_order(1, 1, 100, 40);
    let taker_order = buy_order(2, 2, 100, 50);
    let maker_hash = order_hash(&maker_order);
    let taker_hash = order_hash(&taker_order);

    engine
        .submit_order(signed_submit(maker_order, 100, 1))
        .unwrap();
    let result = engine
        .submit_order(signed_submit(taker_order, 101, 2).with_reservation_ttl_secs(Some(10)))
        .unwrap();
    let reservation_id = match result.outcome {
        SubmitOrderOutcome::Matched { reservation_id, .. } => reservation_id,
        other => panic!("expected match, got {other:?}"),
    };

    (engine, maker_hash, taker_hash, reservation_id)
}

#[test]
fn persists_snapshot_and_recovers_engine() {
    let (engine, maker_hash, taker_hash, reservation_id) = matched_engine();
    let mut store = InMemoryEngineStore::new();

    store
        .persist_engine_snapshot(engine.snapshot(), 1_000)
        .unwrap();
    let mut recovered = store.recover_engine(MatchConfig::default()).unwrap();

    assert_eq!(store.load_orders().unwrap().len(), 2);
    assert_eq!(store.load_reservations().unwrap().len(), 1);
    assert_eq!(
        recovered.order_record(maker_hash).unwrap().state(),
        OrderState::Reserved
    );
    assert_eq!(
        recovered.order_record(maker_hash).unwrap().signature,
        Some(vec![1; 65])
    );
    assert_eq!(
        recovered.order_record(taker_hash).unwrap().state(),
        OrderState::Reserved
    );
    assert_eq!(
        recovered.reservation(reservation_id).unwrap().status,
        ReservationStatus::Reserved
    );

    recovered.expire_reservation(reservation_id, 111).unwrap();
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
fn appends_events_in_sequence_order() {
    let mut store = InMemoryEngineStore::new();

    store
        .append_event(StoredEngineEvent {
            sequence: 2,
            created_at: 100,
            event: EngineEvent::ReservationCommitted {
                reservation_id: B256::repeat_byte(2),
            },
        })
        .unwrap();
    store
        .append_event(StoredEngineEvent {
            sequence: 1,
            created_at: 99,
            event: EngineEvent::OrderFilled {
                order_hash: B256::repeat_byte(1),
            },
        })
        .unwrap();

    let events = store.load_events().unwrap();
    assert_eq!(events[0].sequence, 1);
    assert_eq!(events[1].sequence, 2);
}

#[test]
fn rejects_duplicate_event_sequence() {
    let mut store = InMemoryEngineStore::new();
    let event = StoredEngineEvent {
        sequence: 1,
        created_at: 100,
        event: EngineEvent::OrderFilled {
            order_hash: B256::repeat_byte(1),
        },
    };

    store.append_event(event.clone()).unwrap();
    assert_eq!(
        store.append_event(event),
        Err(StorageError::DuplicateEventSequence(1))
    );
}

#[test]
fn rejects_event_sequence_overflow_before_appending() {
    let mut store = InMemoryEngineStore::new();
    let events = vec![
        EngineEvent::OrderFilled {
            order_hash: B256::repeat_byte(1),
        },
        EngineEvent::OrderFilled {
            order_hash: B256::repeat_byte(2),
        },
    ];

    assert_eq!(
        store.append_engine_events(u64::MAX, 100, &events),
        Err(StorageError::SequenceOverflow)
    );
    assert!(store.load_events().unwrap().is_empty());
}

#[test]
fn appends_fills_in_sequence_order() {
    let mut store = InMemoryEngineStore::new();
    let reservation_id = B256::repeat_byte(9);

    store
        .append_fill(StoredFill {
            sequence: 2,
            reservation_id,
            order_hash: B256::repeat_byte(2),
            claim_amount: U256::from(5),
            new_filled_claim_amount: U256::from(10),
            created_at: 100,
        })
        .unwrap();
    store
        .append_fill(StoredFill {
            sequence: 1,
            reservation_id,
            order_hash: B256::repeat_byte(1),
            claim_amount: U256::from(5),
            new_filled_claim_amount: U256::from(5),
            created_at: 99,
        })
        .unwrap();

    let fills = store.load_fills().unwrap();
    assert_eq!(fills[0].sequence, 1);
    assert_eq!(fills[1].sequence, 2);
}
