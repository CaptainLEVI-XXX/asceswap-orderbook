use asceswap_types::{OrderHash, U256};

use crate::{OrderState, ReservationId, ReservationStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateError {
    InvalidOrderTransition {
        from: OrderState,
        to: OrderState,
    },
    DuplicateReservation(ReservationId),
    MissingReservation(ReservationId),
    ReservationNotActive {
        reservation_id: ReservationId,
        status: ReservationStatus,
    },
    ReservationNotExpired {
        reservation_id: ReservationId,
        expires_at: Option<u64>,
        now: u64,
    },
    InvalidReservationExpiry {
        created_at: u64,
        expires_at: u64,
    },
    MissingAvailability(OrderHash),
    InsufficientAvailable {
        order_hash: OrderHash,
        requested: U256,
        available: U256,
    },
    ArithmeticOverflow,
}
