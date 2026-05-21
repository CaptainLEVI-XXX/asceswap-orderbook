use asceswap_math::remaining_claim_amount;
use asceswap_types::{Order, OrderHash, U256};

use crate::{order_hash, OrderValidationContext, SignatureCheck, ValidationError};

pub const MAX_EXCHANGE_FEE_RATE_BPS: u16 = 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedOrder {
    pub order_hash: OrderHash,
    pub filled_claim_amount: U256,
    pub remaining_claim_amount: U256,
}

pub fn validate_order(
    order: &Order,
    context: &OrderValidationContext,
) -> Result<ValidatedOrder, ValidationError> {
    order.validate_basic()?;

    let actual_hash = order_hash(order);
    if let Some(expected_hash) = context.expected_order_hash {
        if expected_hash != actual_hash {
            return Err(ValidationError::OrderHashMismatch {
                expected: expected_hash,
                actual: actual_hash,
            });
        }
    }

    if order.expiration != U256::ZERO && U256::from(context.now) > order.expiration {
        return Err(ValidationError::Expired {
            expiration: order.expiration,
            now: context.now,
        });
    }

    if context.cancelled {
        return Err(ValidationError::Cancelled);
    }

    if order.epoch != context.maker_epoch {
        return Err(ValidationError::EpochMismatch {
            order_epoch: order.epoch,
            maker_epoch: context.maker_epoch,
        });
    }

    if context.fee_rate_bps > MAX_EXCHANGE_FEE_RATE_BPS {
        return Err(ValidationError::InvalidExchangeFeeRate {
            fee_rate_bps: context.fee_rate_bps,
            max_fee_rate_bps: MAX_EXCHANGE_FEE_RATE_BPS,
        });
    }

    if context.fee_rate_bps > order.max_fee_rate_bps {
        return Err(ValidationError::FeeRateTooHigh {
            fee_rate_bps: context.fee_rate_bps,
            max_fee_rate_bps: order.max_fee_rate_bps,
        });
    }

    let remaining = remaining_claim_amount(order, context.filled_claim_amount)?;
    if remaining == U256::ZERO {
        return Err(ValidationError::NoRemainingClaim);
    }

    if context.signature == SignatureCheck::Invalid {
        return Err(ValidationError::InvalidSignature);
    }

    if context.require_signature && context.signature != SignatureCheck::Valid {
        return Err(ValidationError::MissingSignatureVerification);
    }

    Ok(ValidatedOrder {
        order_hash: actual_hash,
        filled_claim_amount: context.filled_claim_amount,
        remaining_claim_amount: remaining,
    })
}
