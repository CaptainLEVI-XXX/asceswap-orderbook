use serde::{Deserialize, Serialize};

use asceswap_engine::{CancelOrder, SubmitOrder};
use asceswap_validation::OrderValidationContext;

use crate::wire::{parse_b256, parse_u256, ApiClaimSide, ApiOrder, ApiSide, ApiSignatureCheck};
use crate::ApiError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationContextRequest {
    pub now: u64,
    pub expected_order_hash: Option<String>,
    pub filled_claim_amount: String,
    pub cancelled: bool,
    pub maker_epoch: String,
    pub fee_rate_bps: u16,
    pub signature: ApiSignatureCheck,
    pub require_signature: bool,
}

impl ValidationContextRequest {
    pub fn to_context(&self) -> Result<OrderValidationContext, ApiError> {
        let mut context = OrderValidationContext::new(self.now)
            .with_filled_claim_amount(parse_u256(
                "filled_claim_amount",
                &self.filled_claim_amount,
            )?)
            .with_cancelled(self.cancelled)
            .with_maker_epoch(parse_u256("maker_epoch", &self.maker_epoch)?)
            .with_fee_rate_bps(self.fee_rate_bps)
            .with_signature(self.signature.into())
            .with_required_signature(self.require_signature);

        if let Some(expected_order_hash) = &self.expected_order_hash {
            context = context
                .with_expected_order_hash(parse_b256("expected_order_hash", expected_order_hash)?);
        }

        Ok(context)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitOrderRequest {
    pub order: ApiOrder,
    pub validation: ValidationContextRequest,
    pub rest_on_no_match: bool,
    pub reservation_ttl_secs: Option<u64>,
}

impl SubmitOrderRequest {
    pub fn to_command(&self) -> Result<SubmitOrder, ApiError> {
        Ok(
            SubmitOrder::new(self.order.to_order()?, self.validation.to_context()?)
                .with_rest_on_no_match(self.rest_on_no_match)
                .with_reservation_ttl_secs(self.reservation_ttl_secs),
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelOrderRequest {
    pub order_hash: String,
    pub now: u64,
}

impl CancelOrderRequest {
    pub fn to_command(&self) -> Result<CancelOrder, ApiError> {
        Ok(CancelOrder::new(parse_b256(
            "order_hash",
            &self.order_hash,
        )?))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservationActionRequest {
    pub reservation_id: String,
    pub now: u64,
}

impl ReservationActionRequest {
    pub fn reservation_id(&self) -> Result<asceswap_state::ReservationId, ApiError> {
        parse_b256("reservation_id", &self.reservation_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderStatusRequest {
    pub order_hash: String,
}

impl OrderStatusRequest {
    pub fn order_hash(&self) -> Result<asceswap_types::OrderHash, ApiError> {
        parse_b256("order_hash", &self.order_hash)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketDepthRequest {
    pub market_id: String,
    pub claim: ApiClaimSide,
    pub side: ApiSide,
}

impl MarketDepthRequest {
    pub fn market_id(&self) -> Result<asceswap_types::MarketId, ApiError> {
        parse_b256("market_id", &self.market_id)
    }
}
