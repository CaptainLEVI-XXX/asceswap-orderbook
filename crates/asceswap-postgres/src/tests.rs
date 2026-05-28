use std::time::{SystemTime, UNIX_EPOCH};

use asceswap_engine::{
    AsceSwapEngine, EngineEvent, EngineSnapshot, SubmitOrder, SubmitOrderOutcome,
};
use asceswap_matcher::MatchConfig;
use asceswap_math::MathError;
use asceswap_state::{OrderState, ReservationLegRole, ReservationStatus};
use asceswap_storage::EngineStore;
use asceswap_types::{Address, ClaimSide, MatchKind, Order, OrderError, Side, B256, U256};
use asceswap_validation::{order_hash, OrderValidationContext, SignatureCheck, ValidationError};
use postgres::{Client, NoTls};

use crate::codec::{
    b256_from_bytes, decode_event, encode_event, i64_to_u64, u256_from_string, u64_to_i64,
};
use crate::{PostgresEngineStore, POSTGRES_SCHEMA};

fn hash(byte: u8) -> B256 {
    B256::repeat_byte(byte)
}

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

fn with_postgres_store(test: impl FnOnce(&mut PostgresEngineStore)) {
    let url = std::env::var("ASCESWAP_POSTGRES_URL")
        .expect("set ASCESWAP_POSTGRES_URL to run live Postgres tests");
    let schema = TestSchema {
        url: url.clone(),
        name: unique_schema_name(),
    };
    let mut client = Client::connect(&url, NoTls).unwrap();
    client
        .batch_execute(&format!(
            "CREATE SCHEMA {}; SET search_path TO {};",
            schema.name, schema.name
        ))
        .unwrap();

    let mut store = PostgresEngineStore::new(client);
    store.run_schema().unwrap();
    store.run_schema().unwrap();
    test(&mut store);
}

struct TestSchema {
    url: String,
    name: String,
}

impl Drop for TestSchema {
    fn drop(&mut self) {
        if let Ok(mut client) = Client::connect(&self.url, NoTls) {
            let _ = client.batch_execute(&format!("DROP SCHEMA IF EXISTS {} CASCADE;", self.name));
        }
    }
}

fn unique_schema_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("asceswap_test_{}_{}", std::process::id(), nanos)
}

fn assert_event_round_trip(event: EngineEvent) {
    let encoded = encode_event(&event);
    let decoded = decode_event(encoded.kind, &encoded.payload).unwrap();
    assert_eq!(decoded, event);
}

#[test]
fn event_codec_round_trips_engine_events() {
    let events = vec![
        EngineEvent::OrderReceived {
            order_hash: hash(1),
            market_id: hash(2),
        },
        EngineEvent::OrderValidated {
            order_hash: hash(1),
            remaining_claim_amount: U256::from(100),
        },
        EngineEvent::OrderRejected {
            order_hash: hash(1),
            reason: ValidationError::OrderHashMismatch {
                expected: hash(3),
                actual: hash(4),
            },
        },
        EngineEvent::OrderRejected {
            order_hash: hash(1),
            reason: ValidationError::Fill(MathError::Order(OrderError::ImpossiblePrice)),
        },
        EngineEvent::OrderOpened {
            order_hash: hash(1),
        },
        EngineEvent::OrderInactive {
            order_hash: hash(1),
        },
        EngineEvent::OrderReserved {
            order_hash: hash(1),
            reservation_id: hash(9),
        },
        EngineEvent::OrderSubmitted {
            order_hash: hash(1),
            reservation_id: hash(9),
        },
        EngineEvent::OrderStateChanged {
            order_hash: hash(1),
            state: OrderState::PartiallyFilled,
        },
        EngineEvent::OrderPartiallyFilled {
            order_hash: hash(1),
            filled_claim_amount: U256::from(40),
            remaining_claim_amount: U256::from(60),
        },
        EngineEvent::OrderFilled {
            order_hash: hash(1),
        },
        EngineEvent::OrderCancelled {
            order_hash: hash(1),
        },
        EngineEvent::ReservationCreated {
            reservation_id: hash(9),
            match_kind: MatchKind::MergeAssisted,
            maker_count: 32,
        },
        EngineEvent::ReservationSubmitted {
            reservation_id: hash(9),
        },
        EngineEvent::ReservationReleased {
            reservation_id: hash(9),
        },
        EngineEvent::ReservationExpired {
            reservation_id: hash(9),
        },
        EngineEvent::ReservationCommitted {
            reservation_id: hash(9),
        },
    ];

    for event in events {
        assert_event_round_trip(event);
    }
}

#[test]
fn codec_rejects_out_of_range_storage_values() {
    assert!(u64_to_i64("sequence", u64::MAX).is_err());
    assert!(i64_to_u64("sequence", -1).is_err());
    assert!(b256_from_bytes("order_hash", vec![0; 31]).is_err());
    assert!(u256_from_string("amount", "not-a-number").is_err());
}

#[test]
fn postgres_schema_preserves_reservation_leg_order_and_payloads() {
    assert!(POSTGRES_SCHEMA.contains("leg_index INTEGER NOT NULL"));
    assert!(POSTGRES_SCHEMA.contains("accepted_sequence BIGINT"));
    assert!(POSTGRES_SCHEMA.contains("payload JSONB NOT NULL"));
    assert!(POSTGRES_SCHEMA.contains("NUMERIC(78, 0)"));
}

#[test]
#[ignore = "requires ASCESWAP_POSTGRES_URL"]
fn live_postgres_round_trips_snapshot_events_and_sequence() {
    with_postgres_store(|store| {
        let mut engine = AsceSwapEngine::default();
        let maker = sell_order(1, 1, 100, 40);
        let taker = buy_order(2, 2, 100, 50);
        let maker_hash = order_hash(&maker);
        let taker_hash = order_hash(&taker);

        let first = engine.submit_order(submit(maker, 100)).unwrap();
        store
            .persist_engine_update(0, 100, &first.events, engine.snapshot())
            .unwrap();
        let second = engine
            .submit_order(submit(taker, 101).with_reservation_ttl_secs(Some(10)))
            .unwrap();
        let reservation_id = match second.outcome {
            SubmitOrderOutcome::Matched { reservation_id, .. } => reservation_id,
            other => panic!("expected matched outcome, got {other:?}"),
        };
        store
            .persist_engine_update(
                first.events.len() as u64,
                101,
                &second.events,
                engine.snapshot(),
            )
            .unwrap();

        let expected_last_sequence = first.events.len() as u64 + second.events.len() as u64 - 1;
        assert_eq!(
            store.last_event_sequence().unwrap(),
            Some(expected_last_sequence)
        );
        assert_eq!(
            store.load_reservations().unwrap()[0].reservation.legs[0].role,
            ReservationLegRole::Taker
        );

        let recovered = store.recover_engine(MatchConfig::default()).unwrap();
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
    });
}

#[test]
#[ignore = "requires ASCESWAP_POSTGRES_URL"]
fn live_postgres_rolls_back_events_when_snapshot_write_fails() {
    with_postgres_store(|store| {
        let bad_snapshot = EngineSnapshot {
            orders: Vec::new(),
            reservations: Vec::new(),
            next_reservation_sequence: u64::MAX,
        };
        let events = vec![EngineEvent::OrderFilled {
            order_hash: hash(1),
        }];

        assert!(store
            .persist_engine_update(0, 100, &events, bad_snapshot)
            .is_err());
        assert!(store.load_events().unwrap().is_empty());
        assert!(store.load_snapshot().unwrap().is_none());
    });
}
