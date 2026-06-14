use serde::{Deserialize, Serialize};

use crate::event::ApiEvent;
use crate::wire::{
    ApiClaimSide, ApiMatchKind, ApiOrder, ApiOrderState, ApiReservationLegRole,
    ApiReservationStatus, ApiSide,
};

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
    PostOnlyWouldCross,
    Inactive,
    Matched {
        reservation_id: String,
        match_kind: ApiMatchKind,
        taker_claim_fill_amount: String,
        maker_count: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        settlement: Option<SettlementPayloadResponse>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementPayloadResponse {
    pub taker_order: ApiOrder,
    pub taker_signature: String,
    pub maker_orders: Vec<ApiOrder>,
    pub maker_signatures: Vec<String>,
    pub taker_claim_fill_amount: String,
    pub maker_claim_fill_amounts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelOrderResponse {
    pub order_hash: String,
    pub events: Vec<ApiEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservationActionResponse {
    pub reservation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderSummaryResponse {
    pub order_hash: String,
    pub order: ApiOrder,
    pub state: ApiOrderState,
    pub filled_claim_amount: String,
    pub remaining_claim_amount: String,
    pub resting: bool,
    pub accepted_sequence: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListOrdersResponse {
    pub orders: Vec<OrderSummaryResponse>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketSummaryResponse {
    pub market_id: String,
    pub order_count: usize,
    pub resting_order_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListMarketsResponse {
    pub markets: Vec<MarketSummaryResponse>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListEventsResponse {
    pub events: Vec<ApiEvent>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservationLegResponse {
    pub order_hash: String,
    pub role: ApiReservationLegRole,
    pub claim_amount: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservationSummaryResponse {
    pub reservation_id: String,
    pub status: ApiReservationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    pub created_at: u64,
    pub expires_at: Option<u64>,
    pub updated_at: u64,
    pub legs: Vec<ReservationLegResponse>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListReservationsResponse {
    pub reservations: Vec<ReservationSummaryResponse>,
}
