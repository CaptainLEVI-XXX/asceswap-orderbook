use std::collections::HashMap;

use asceswap_matcher::{MakerFill, MatchPlan};
use asceswap_types::{MatchKind, OrderHash, B256, U256};

use crate::{
    derive_reservation_id, is_allowed_transition, OrderAvailability, OrderLifecycle, OrderState,
    ReservationBook, ReservationLegRole, ReservationStatus, StateError,
};

fn hash(value: u8) -> OrderHash {
    B256::repeat_byte(value)
}

fn plan() -> MatchPlan {
    MatchPlan {
        match_kind: MatchKind::Direct,
        taker_claim_fill_amount: U256::from(100),
        taker_collateral_amount: U256::from(50),
        taker_actual_collateral_amount: U256::from(49),
        total_maker_claim_fill_amount: U256::from(100),
        total_maker_collateral_amount: U256::from(49),
        maker_fills: vec![MakerFill {
            order_hash: hash(2),
            claim_fill_amount: U256::from(100),
            collateral_amount: U256::from(49),
            new_filled_claim_amount: U256::from(100),
        }],
    }
}

fn availability() -> HashMap<OrderHash, OrderAvailability> {
    HashMap::from([
        (
            hash(1),
            OrderAvailability {
                filled_claim_amount: U256::ZERO,
                max_claim_amount: U256::from(100),
            },
        ),
        (
            hash(2),
            OrderAvailability {
                filled_claim_amount: U256::ZERO,
                max_claim_amount: U256::from(100),
            },
        ),
    ])
}

fn large_availability() -> HashMap<OrderHash, OrderAvailability> {
    HashMap::from([
        (
            hash(1),
            OrderAvailability {
                filled_claim_amount: U256::ZERO,
                max_claim_amount: U256::from(300),
            },
        ),
        (
            hash(2),
            OrderAvailability {
                filled_claim_amount: U256::ZERO,
                max_claim_amount: U256::from(300),
            },
        ),
    ])
}

#[test]
fn applies_allowed_order_lifecycle_transitions() {
    let mut lifecycle = OrderLifecycle::new(OrderState::Received);

    let transition = lifecycle.transition_to(OrderState::Validating).unwrap();
    assert_eq!(transition.from, OrderState::Received);
    assert_eq!(transition.to, OrderState::Validating);
    assert_eq!(lifecycle.state(), OrderState::Validating);

    lifecycle.transition_to(OrderState::Open).unwrap();
    lifecycle.transition_to(OrderState::Reserved).unwrap();
    lifecycle.transition_to(OrderState::Submitted).unwrap();
    lifecycle.transition_to(OrderState::Filled).unwrap();
}

#[test]
fn rejects_forbidden_order_lifecycle_transitions() {
    assert!(!is_allowed_transition(OrderState::Filled, OrderState::Open));

    let mut lifecycle = OrderLifecycle::new(OrderState::Filled);
    assert_eq!(
        lifecycle.transition_to(OrderState::Open),
        Err(StateError::InvalidOrderTransition {
            from: OrderState::Filled,
            to: OrderState::Open,
        })
    );
}

#[test]
fn creates_reservation_legs_and_tracks_reserved_amounts() {
    let mut book = ReservationBook::new();
    let reservation = book
        .create(B256::repeat_byte(9), hash(1), &plan(), &availability())
        .unwrap();

    assert_eq!(reservation.status, ReservationStatus::Reserved);
    assert_eq!(reservation.legs.len(), 2);
    assert_eq!(reservation.legs[0].role, ReservationLegRole::Taker);
    assert_eq!(reservation.legs[1].role, ReservationLegRole::Maker);
    assert_eq!(book.reserved_claim_amount(hash(1)), U256::from(100));
    assert_eq!(book.reserved_claim_amount(hash(2)), U256::from(100));
}

#[test]
fn rebuilds_reservation_book_from_active_reservations() {
    let mut book = ReservationBook::new();
    let reservation_id = B256::repeat_byte(9);
    let reservation = book
        .create(reservation_id, hash(1), &plan(), &availability())
        .unwrap()
        .clone();

    let rebuilt = ReservationBook::from_reservations(vec![reservation]).unwrap();

    assert_eq!(rebuilt.reserved_claim_amount(hash(1)), U256::from(100));
    assert_eq!(rebuilt.reserved_claim_amount(hash(2)), U256::from(100));
    assert_eq!(
        rebuilt.get(reservation_id).unwrap().status,
        ReservationStatus::Reserved
    );
}

#[test]
fn rejects_invalid_expiry_when_rebuilding_reservation_book() {
    let mut book = ReservationBook::new();
    let mut reservation = book
        .create_with_expiration(
            B256::repeat_byte(9),
            hash(1),
            &plan(),
            &availability(),
            100,
            Some(110),
        )
        .unwrap()
        .clone();
    reservation.expires_at = Some(100);

    assert_eq!(
        ReservationBook::from_reservations(vec![reservation]).unwrap_err(),
        StateError::InvalidReservationExpiry {
            created_at: 100,
            expires_at: 100,
        }
    );
}

#[test]
fn derives_stable_reservation_ids_from_plan_content() {
    let plan = plan();
    let first = derive_reservation_id(hash(1), &plan, 42);
    let second = derive_reservation_id(hash(1), &plan, 42);
    let changed_sequence = derive_reservation_id(hash(1), &plan, 43);
    let changed_taker = derive_reservation_id(hash(9), &plan, 42);

    assert_eq!(first, second);
    assert_ne!(first, changed_sequence);
    assert_ne!(first, changed_taker);
}

#[test]
fn prevents_double_reservation_of_same_available_size() {
    let mut book = ReservationBook::new();
    book.create(B256::repeat_byte(9), hash(1), &plan(), &availability())
        .unwrap();

    assert!(matches!(
        book.create(B256::repeat_byte(10), hash(1), &plan(), &availability()),
        Err(StateError::InsufficientAvailable { .. })
    ));
}

#[test]
fn rejects_invalid_reservation_expiry() {
    let mut book = ReservationBook::new();

    assert_eq!(
        book.create_with_expiration(
            B256::repeat_byte(9),
            hash(1),
            &plan(),
            &availability(),
            100,
            Some(100),
        ),
        Err(StateError::InvalidReservationExpiry {
            created_at: 100,
            expires_at: 100,
        })
    );
}

#[test]
fn release_restores_reserved_amounts() {
    let mut book = ReservationBook::new();
    let reservation_id = B256::repeat_byte(9);
    book.create(reservation_id, hash(1), &plan(), &availability())
        .unwrap();

    let reservation = book.release(reservation_id).unwrap();
    assert_eq!(reservation.status, ReservationStatus::Released);
    assert_eq!(book.reserved_claim_amount(hash(1)), U256::ZERO);
    assert_eq!(book.reserved_claim_amount(hash(2)), U256::ZERO);
}

#[test]
fn expiry_releases_reserved_amounts() {
    let mut book = ReservationBook::new();
    let reservation_id = B256::repeat_byte(9);
    book.create_with_expiration(
        reservation_id,
        hash(1),
        &plan(),
        &availability(),
        100,
        Some(110),
    )
    .unwrap();

    assert_eq!(
        book.expire(reservation_id, 109),
        Err(StateError::ReservationNotExpired {
            reservation_id,
            expires_at: Some(110),
            now: 109,
        })
    );

    let reservation = book.expire(reservation_id, 110).unwrap();
    assert_eq!(reservation.status, ReservationStatus::Expired);
    assert_eq!(book.reserved_claim_amount(hash(1)), U256::ZERO);
    assert_eq!(book.reserved_claim_amount(hash(2)), U256::ZERO);
}

#[test]
fn mark_submitted_at_expires_stale_reserved_order() {
    let mut book = ReservationBook::new();
    let reservation_id = B256::repeat_byte(9);
    book.create_with_expiration(
        reservation_id,
        hash(1),
        &plan(),
        &availability(),
        100,
        Some(110),
    )
    .unwrap();

    assert_eq!(
        book.mark_submitted_at(reservation_id, 110),
        Err(StateError::ReservationNotActive {
            reservation_id,
            status: ReservationStatus::Expired,
        })
    );
    assert_eq!(
        book.get(reservation_id).unwrap().status,
        ReservationStatus::Expired
    );
    assert_eq!(book.reserved_claim_amount(hash(1)), U256::ZERO);
}

#[test]
fn expire_expired_only_releases_expired_reserved_orders() {
    let mut book = ReservationBook::new();
    let expired_id = B256::repeat_byte(9);
    let live_id = B256::repeat_byte(10);
    let availability = large_availability();

    book.create_with_expiration(expired_id, hash(1), &plan(), &availability, 100, Some(110))
        .unwrap();
    book.create_with_expiration(live_id, hash(1), &plan(), &availability, 100, Some(120))
        .unwrap();

    let expired_ids = book.expire_expired(110).unwrap();

    assert_eq!(expired_ids, vec![expired_id]);
    assert_eq!(
        book.get(expired_id).unwrap().status,
        ReservationStatus::Expired
    );
    assert_eq!(
        book.get(live_id).unwrap().status,
        ReservationStatus::Reserved
    );
    assert_eq!(book.reserved_claim_amount(hash(1)), U256::from(100));
    assert_eq!(book.reserved_claim_amount(hash(2)), U256::from(100));
}

#[test]
fn submitted_reservation_can_be_committed() {
    let mut book = ReservationBook::new();
    let reservation_id = B256::repeat_byte(9);
    book.create(reservation_id, hash(1), &plan(), &availability())
        .unwrap();

    assert_eq!(
        book.mark_submitted(reservation_id).unwrap().status,
        ReservationStatus::Submitted
    );
    assert_eq!(
        book.commit(reservation_id).unwrap().status,
        ReservationStatus::Committed
    );
    assert_eq!(book.reserved_claim_amount(hash(1)), U256::ZERO);
}

#[test]
fn commit_requires_submitted_reservation() {
    let mut book = ReservationBook::new();
    let reservation_id = B256::repeat_byte(9);
    book.create(reservation_id, hash(1), &plan(), &availability())
        .unwrap();

    assert_eq!(
        book.commit(reservation_id),
        Err(StateError::ReservationNotActive {
            reservation_id,
            status: ReservationStatus::Reserved,
        })
    );
}
