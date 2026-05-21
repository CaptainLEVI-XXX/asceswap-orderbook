use asceswap_math::{remaining_claim_amount, MathError, Price};
use asceswap_types::{Order, OrderHash, U256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestingOrder {
    pub hash: OrderHash,
    pub order: Order,
    pub filled_claim_amount: U256,
    pub accepted_sequence: u64,
    pub price: Price,
}

impl RestingOrder {
    pub fn remaining_claim_amount(&self) -> Result<U256, MathError> {
        remaining_claim_amount(&self.order, self.filled_claim_amount)
    }
}
