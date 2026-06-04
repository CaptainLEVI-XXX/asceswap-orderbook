use asceswap_state::{OrderLifecycle, OrderState};
use asceswap_types::{Order, OrderHash, U256};

use crate::EngineError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderRecord {
    pub hash: OrderHash,
    pub order: Order,
    pub signature: Option<Vec<u8>>,
    lifecycle: OrderLifecycle,
    pub filled_claim_amount: U256,
    pub resting: bool,
}

impl OrderRecord {
    pub fn new(
        hash: OrderHash,
        order: Order,
        state: OrderState,
        filled_claim_amount: U256,
        resting: bool,
    ) -> Self {
        Self {
            hash,
            order,
            signature: None,
            lifecycle: OrderLifecycle::new(state),
            filled_claim_amount,
            resting,
        }
    }

    pub fn with_signature(mut self, signature: Option<Vec<u8>>) -> Self {
        self.signature = signature;
        self
    }

    pub fn state(&self) -> OrderState {
        self.lifecycle.state()
    }

    pub fn transition_to(&mut self, state: OrderState) -> Result<(), EngineError> {
        self.lifecycle.transition_to(state)?;
        Ok(())
    }
}
