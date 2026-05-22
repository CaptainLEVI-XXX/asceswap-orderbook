use asceswap_state::{OrderState, ReservationId};
use asceswap_types::{MarketId, MatchKind, OrderHash, U256};
use asceswap_validation::ValidationError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineEvent {
    OrderReceived {
        order_hash: OrderHash,
        market_id: MarketId,
    },
    OrderValidated {
        order_hash: OrderHash,
        remaining_claim_amount: U256,
    },
    OrderRejected {
        order_hash: OrderHash,
        reason: ValidationError,
    },
    OrderOpened {
        order_hash: OrderHash,
    },
    OrderInactive {
        order_hash: OrderHash,
    },
    OrderReserved {
        order_hash: OrderHash,
        reservation_id: ReservationId,
    },
    OrderSubmitted {
        order_hash: OrderHash,
        reservation_id: ReservationId,
    },
    OrderStateChanged {
        order_hash: OrderHash,
        state: OrderState,
    },
    OrderPartiallyFilled {
        order_hash: OrderHash,
        filled_claim_amount: U256,
        remaining_claim_amount: U256,
    },
    OrderFilled {
        order_hash: OrderHash,
    },
    OrderCancelled {
        order_hash: OrderHash,
    },
    ReservationCreated {
        reservation_id: ReservationId,
        match_kind: MatchKind,
        maker_count: usize,
    },
    ReservationSubmitted {
        reservation_id: ReservationId,
    },
    ReservationReleased {
        reservation_id: ReservationId,
    },
    ReservationExpired {
        reservation_id: ReservationId,
    },
    ReservationCommitted {
        reservation_id: ReservationId,
    },
}
