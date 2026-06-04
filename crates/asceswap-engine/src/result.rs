use asceswap_matcher::MatchPlan;
use asceswap_math::Price;
use asceswap_state::ReservationId;
use asceswap_types::OrderHash;
use asceswap_validation::ValidationError;

use crate::EngineEvent;
use crate::SettlementPayload;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmitOrderResult {
    pub order_hash: OrderHash,
    pub outcome: SubmitOrderOutcome,
    pub events: Vec<EngineEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmitOrderOutcome {
    Rejected {
        reason: ValidationError,
    },
    Rested {
        price: Price,
    },
    Inactive,
    Matched {
        reservation_id: ReservationId,
        plan: MatchPlan,
        settlement: Option<SettlementPayload>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CancelOrderResult {
    pub order_hash: OrderHash,
    pub events: Vec<EngineEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReservationUpdateResult {
    pub reservation_id: ReservationId,
    pub events: Vec<EngineEvent>,
}
