use asceswap_engine::{EngineEvent, EngineSnapshot, OrderSnapshot};
use asceswap_state::{Reservation, ReservationId};
use asceswap_types::{OrderHash, U256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredOrder {
    pub snapshot: OrderSnapshot,
    pub created_at: u64,
    pub updated_at: u64,
}

impl StoredOrder {
    pub fn from_snapshot(snapshot: OrderSnapshot, now: u64) -> Self {
        Self {
            snapshot,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn hash(&self) -> OrderHash {
        self.snapshot.hash
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredReservation {
    pub reservation: Reservation,
    pub created_at: u64,
    pub updated_at: u64,
}

impl StoredReservation {
    pub fn from_reservation(reservation: Reservation, now: u64) -> Self {
        Self {
            reservation,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn id(&self) -> ReservationId {
        self.reservation.id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredFill {
    pub sequence: u64,
    pub reservation_id: ReservationId,
    pub order_hash: OrderHash,
    pub claim_amount: U256,
    pub new_filled_claim_amount: U256,
    pub created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredEngineEvent {
    pub sequence: u64,
    pub created_at: u64,
    pub event: EngineEvent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredSnapshot {
    pub engine: EngineSnapshot,
    pub created_at: u64,
}
