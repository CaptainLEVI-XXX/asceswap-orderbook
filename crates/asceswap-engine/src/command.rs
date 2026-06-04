use asceswap_types::{Order, OrderHash};
use asceswap_validation::OrderValidationContext;

#[derive(Clone, Debug)]
pub struct SubmitOrder {
    pub order: Order,
    pub validation: OrderValidationContext,
    pub signature: Option<Vec<u8>>,
    pub rest_on_no_match: bool,
    pub reservation_ttl_secs: Option<u64>,
}

impl SubmitOrder {
    pub fn new(order: Order, validation: OrderValidationContext) -> Self {
        Self {
            order,
            validation,
            signature: None,
            rest_on_no_match: true,
            reservation_ttl_secs: None,
        }
    }

    pub fn with_signature(mut self, signature: Option<Vec<u8>>) -> Self {
        self.signature = signature;
        self
    }

    pub fn with_rest_on_no_match(mut self, rest_on_no_match: bool) -> Self {
        self.rest_on_no_match = rest_on_no_match;
        self
    }

    pub fn with_reservation_ttl_secs(mut self, reservation_ttl_secs: Option<u64>) -> Self {
        self.reservation_ttl_secs = reservation_ttl_secs;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CancelOrder {
    pub order_hash: OrderHash,
}

impl CancelOrder {
    pub fn new(order_hash: OrderHash) -> Self {
        Self { order_hash }
    }
}
