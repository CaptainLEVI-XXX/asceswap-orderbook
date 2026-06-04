use serde::{Deserialize, Serialize};

use asceswap_engine::{CancelOrder, SubmitOrder};
use asceswap_validation::{
    verify_order_eoa_signature, OrderValidationContext, SignatureCheck, SignatureDomain,
};

use crate::wire::{
    parse_b256, parse_hex_bytes, parse_u256, ApiClaimSide, ApiOrder, ApiSide, ApiSignatureCheck,
};
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature_bytes: Option<String>,
    #[serde(default)]
    pub post_only: bool,
    pub rest_on_no_match: bool,
    pub reservation_ttl_secs: Option<u64>,
}

impl SubmitOrderRequest {
    pub fn to_command(&self) -> Result<SubmitOrder, ApiError> {
        self.to_command_with_signature_domain(None)
    }

    pub fn to_command_with_signature_domain(
        &self,
        signature_domain: Option<SignatureDomain>,
    ) -> Result<SubmitOrder, ApiError> {
        let order = self.order.to_order()?;
        let mut context = self.validation.to_context()?;
        let signature = self
            .signature_bytes
            .as_deref()
            .map(|value| parse_hex_bytes("signature_bytes", value))
            .transpose()?;

        if let Some(domain) = signature_domain {
            let signature_check = signature
                .as_deref()
                .map(|signature| verify_order_eoa_signature(&order, domain, signature))
                .map(|valid| {
                    if valid {
                        SignatureCheck::Valid
                    } else {
                        SignatureCheck::Invalid
                    }
                })
                .unwrap_or(SignatureCheck::Unchecked);

            context = context
                .with_signature(signature_check)
                .with_required_signature(true);
        }

        Ok(SubmitOrder::new(order, context)
            .with_signature(signature)
            .with_post_only(self.post_only)
            .with_rest_on_no_match(self.rest_on_no_match)
            .with_reservation_ttl_secs(self.reservation_ttl_secs))
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
pub struct SettlementPayloadRequest {
    pub reservation_id: String,
}

impl SettlementPayloadRequest {
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
