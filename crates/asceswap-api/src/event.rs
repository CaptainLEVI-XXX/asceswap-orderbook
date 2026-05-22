use serde::{Deserialize, Serialize};

use asceswap_engine::EngineEvent;

use crate::wire::{encode_b256, encode_u256, ApiMatchKind, ApiOrderState};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiEventKind {
    OrderReceived,
    OrderValidated,
    OrderRejected,
    OrderOpened,
    OrderInactive,
    OrderReserved,
    OrderSubmitted,
    OrderStateChanged,
    OrderPartiallyFilled,
    OrderFilled,
    OrderCancelled,
    ReservationCreated,
    ReservationSubmitted,
    ReservationReleased,
    ReservationExpired,
    ReservationCommitted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiEvent {
    pub sequence: u64,
    pub kind: ApiEventKind,
    pub order_hash: Option<String>,
    pub reservation_id: Option<String>,
    pub market_id: Option<String>,
    pub state: Option<ApiOrderState>,
    pub match_kind: Option<ApiMatchKind>,
    pub claim_amount: Option<String>,
    pub remaining_claim_amount: Option<String>,
    pub maker_count: Option<usize>,
    pub reason: Option<String>,
}

impl ApiEvent {
    pub fn from_engine(sequence: u64, event: &EngineEvent) -> Self {
        let mut out = Self::empty(sequence);

        match event {
            EngineEvent::OrderReceived {
                order_hash,
                market_id,
            } => {
                out.kind = ApiEventKind::OrderReceived;
                out.order_hash = Some(encode_b256(*order_hash));
                out.market_id = Some(encode_b256(*market_id));
            }
            EngineEvent::OrderValidated {
                order_hash,
                remaining_claim_amount,
            } => {
                out.kind = ApiEventKind::OrderValidated;
                out.order_hash = Some(encode_b256(*order_hash));
                out.remaining_claim_amount = Some(encode_u256(*remaining_claim_amount));
            }
            EngineEvent::OrderRejected { order_hash, reason } => {
                out.kind = ApiEventKind::OrderRejected;
                out.order_hash = Some(encode_b256(*order_hash));
                out.reason = Some(format!("{reason:?}"));
            }
            EngineEvent::OrderOpened { order_hash } => {
                out.kind = ApiEventKind::OrderOpened;
                out.order_hash = Some(encode_b256(*order_hash));
            }
            EngineEvent::OrderInactive { order_hash } => {
                out.kind = ApiEventKind::OrderInactive;
                out.order_hash = Some(encode_b256(*order_hash));
            }
            EngineEvent::OrderReserved {
                order_hash,
                reservation_id,
            } => {
                out.kind = ApiEventKind::OrderReserved;
                out.order_hash = Some(encode_b256(*order_hash));
                out.reservation_id = Some(encode_b256(*reservation_id));
            }
            EngineEvent::OrderSubmitted {
                order_hash,
                reservation_id,
            } => {
                out.kind = ApiEventKind::OrderSubmitted;
                out.order_hash = Some(encode_b256(*order_hash));
                out.reservation_id = Some(encode_b256(*reservation_id));
            }
            EngineEvent::OrderStateChanged { order_hash, state } => {
                out.kind = ApiEventKind::OrderStateChanged;
                out.order_hash = Some(encode_b256(*order_hash));
                out.state = Some((*state).into());
            }
            EngineEvent::OrderPartiallyFilled {
                order_hash,
                filled_claim_amount,
                remaining_claim_amount,
            } => {
                out.kind = ApiEventKind::OrderPartiallyFilled;
                out.order_hash = Some(encode_b256(*order_hash));
                out.claim_amount = Some(encode_u256(*filled_claim_amount));
                out.remaining_claim_amount = Some(encode_u256(*remaining_claim_amount));
            }
            EngineEvent::OrderFilled { order_hash } => {
                out.kind = ApiEventKind::OrderFilled;
                out.order_hash = Some(encode_b256(*order_hash));
            }
            EngineEvent::OrderCancelled { order_hash } => {
                out.kind = ApiEventKind::OrderCancelled;
                out.order_hash = Some(encode_b256(*order_hash));
            }
            EngineEvent::ReservationCreated {
                reservation_id,
                match_kind,
                maker_count,
            } => {
                out.kind = ApiEventKind::ReservationCreated;
                out.reservation_id = Some(encode_b256(*reservation_id));
                out.match_kind = Some((*match_kind).into());
                out.maker_count = Some(*maker_count);
            }
            EngineEvent::ReservationSubmitted { reservation_id } => {
                out.kind = ApiEventKind::ReservationSubmitted;
                out.reservation_id = Some(encode_b256(*reservation_id));
            }
            EngineEvent::ReservationReleased { reservation_id } => {
                out.kind = ApiEventKind::ReservationReleased;
                out.reservation_id = Some(encode_b256(*reservation_id));
            }
            EngineEvent::ReservationExpired { reservation_id } => {
                out.kind = ApiEventKind::ReservationExpired;
                out.reservation_id = Some(encode_b256(*reservation_id));
            }
            EngineEvent::ReservationCommitted { reservation_id } => {
                out.kind = ApiEventKind::ReservationCommitted;
                out.reservation_id = Some(encode_b256(*reservation_id));
            }
        }

        out
    }

    fn empty(sequence: u64) -> Self {
        Self {
            sequence,
            kind: ApiEventKind::OrderReceived,
            order_hash: None,
            reservation_id: None,
            market_id: None,
            state: None,
            match_kind: None,
            claim_amount: None,
            remaining_claim_amount: None,
            maker_count: None,
            reason: None,
        }
    }
}
