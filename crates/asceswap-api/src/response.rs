use serde::{Deserialize, Serialize};

use crate::event::ApiEvent;
use crate::wire::{ApiClaimSide, ApiMatchKind, ApiOrderState, ApiSide};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitOrderResponse {
    pub order_hash: String,
    pub outcome: SubmitOrderResponseOutcome,
    pub events: Vec<ApiEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubmitOrderResponseOutcome {
    Rejected {
        reason: String,
    },
    Rested {
        price_wad: String,
    },
    Inactive,
    Matched {
        reservation_id: String,
        match_kind: ApiMatchKind,
        taker_claim_fill_amount: String,
        maker_count: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelOrderResponse {
    pub order_hash: String,
    pub events: Vec<ApiEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservationActionResponse {
    pub reservation_id: String,
    pub events: Vec<ApiEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderStatusResponse {
    pub order_hash: String,
    pub state: ApiOrderState,
    pub filled_claim_amount: String,
    pub remaining_claim_amount: String,
    pub resting: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepthLevelResponse {
    pub price_wad: String,
    pub total_claim_amount: String,
    pub order_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketDepthResponse {
    pub market_id: String,
    pub claim: ApiClaimSide,
    pub side: ApiSide,
    pub levels: Vec<DepthLevelResponse>,
}
