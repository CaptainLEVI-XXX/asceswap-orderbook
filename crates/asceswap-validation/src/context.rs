use asceswap_types::{OrderHash, U256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureCheck {
    Unchecked,
    Valid,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderValidationContext {
    pub expected_order_hash: Option<OrderHash>,
    pub filled_claim_amount: U256,
    pub cancelled: bool,
    pub maker_epoch: U256,
    pub now: u64,
    pub fee_rate_bps: u16,
    pub signature: SignatureCheck,
    pub require_signature: bool,
}

impl OrderValidationContext {
    pub fn new(now: u64) -> Self {
        Self {
            expected_order_hash: None,
            filled_claim_amount: U256::ZERO,
            cancelled: false,
            maker_epoch: U256::ZERO,
            now,
            fee_rate_bps: 0,
            signature: SignatureCheck::Unchecked,
            require_signature: false,
        }
    }

    pub fn with_expected_order_hash(mut self, expected_order_hash: OrderHash) -> Self {
        self.expected_order_hash = Some(expected_order_hash);
        self
    }

    pub fn with_filled_claim_amount(mut self, filled_claim_amount: U256) -> Self {
        self.filled_claim_amount = filled_claim_amount;
        self
    }

    pub fn with_cancelled(mut self, cancelled: bool) -> Self {
        self.cancelled = cancelled;
        self
    }

    pub fn with_maker_epoch(mut self, maker_epoch: U256) -> Self {
        self.maker_epoch = maker_epoch;
        self
    }

    pub fn with_fee_rate_bps(mut self, fee_rate_bps: u16) -> Self {
        self.fee_rate_bps = fee_rate_bps;
        self
    }

    pub fn with_signature(mut self, signature: SignatureCheck) -> Self {
        self.signature = signature;
        self
    }

    pub fn with_required_signature(mut self, require_signature: bool) -> Self {
        self.require_signature = require_signature;
        self
    }
}
