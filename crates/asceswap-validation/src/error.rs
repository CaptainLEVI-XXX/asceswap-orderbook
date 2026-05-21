use asceswap_math::MathError;
use asceswap_types::{OrderError, OrderHash, U256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationError {
    BasicOrder(OrderError),
    OrderHashMismatch {
        expected: OrderHash,
        actual: OrderHash,
    },
    InvalidSignature,
    Expired {
        expiration: U256,
        now: u64,
    },
    Cancelled,
    EpochMismatch {
        order_epoch: U256,
        maker_epoch: U256,
    },
    FeeRateTooHigh {
        fee_rate_bps: u16,
        max_fee_rate_bps: u16,
    },
    InvalidExchangeFeeRate {
        fee_rate_bps: u16,
        max_fee_rate_bps: u16,
    },
    MissingSignatureVerification,
    Fill(MathError),
    NoRemainingClaim,
}

impl From<OrderError> for ValidationError {
    fn from(error: OrderError) -> Self {
        Self::BasicOrder(error)
    }
}

impl From<MathError> for ValidationError {
    fn from(error: MathError) -> Self {
        Self::Fill(error)
    }
}
