use asceswap_state::{OrderState, Reservation};
use asceswap_types::{Order, OrderHash, U256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderSnapshot {
    pub hash: OrderHash,
    pub order: Order,
    pub state: OrderState,
    pub filled_claim_amount: U256,
    pub resting: bool,
    pub accepted_sequence: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineSnapshot {
    pub orders: Vec<OrderSnapshot>,
    pub reservations: Vec<Reservation>,
    pub next_reservation_sequence: u64,
}
