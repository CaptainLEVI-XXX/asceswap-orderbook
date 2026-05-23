use asceswap_engine::EngineEvent;
use asceswap_math::MathError;
use asceswap_state::OrderState;
use asceswap_types::{MatchKind, OrderError, B256, U256};
use asceswap_validation::ValidationError;

use crate::codec::{
    b256_from_bytes, decode_event, encode_event, i64_to_u64, u256_from_string, u64_to_i64,
};
use crate::POSTGRES_SCHEMA;

fn hash(byte: u8) -> B256 {
    B256::repeat_byte(byte)
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
    assert!(POSTGRES_SCHEMA.contains("payload JSONB NOT NULL"));
    assert!(POSTGRES_SCHEMA.contains("NUMERIC(78, 0)"));
}
