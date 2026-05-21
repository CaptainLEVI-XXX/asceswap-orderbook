use crate::StateError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OrderState {
    Received,
    Validating,
    Rejected,
    Open,
    PartiallyFilled,
    Reserved,
    Submitted,
    Filled,
    Expired,
    SoftCancelled,
    CancelPending,
    Cancelled,
    EpochInvalidated,
    Inactive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderTransition {
    pub from: OrderState,
    pub to: OrderState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderLifecycle {
    state: OrderState,
}

impl OrderLifecycle {
    pub fn new(state: OrderState) -> Self {
        Self { state }
    }

    pub fn state(&self) -> OrderState {
        self.state
    }

    pub fn transition_to(&mut self, to: OrderState) -> Result<OrderTransition, StateError> {
        let from = self.state;
        if !is_allowed_transition(from, to) {
            return Err(StateError::InvalidOrderTransition { from, to });
        }

        self.state = to;
        Ok(OrderTransition { from, to })
    }
}

pub fn is_allowed_transition(from: OrderState, to: OrderState) -> bool {
    use OrderState::*;

    matches!(
        (from, to),
        (Received, Validating)
            | (Validating, Rejected)
            | (Validating, Open)
            | (Validating, Reserved)
            | (Open, Reserved)
            | (Open, PartiallyFilled)
            | (Reserved, PartiallyFilled)
            | (Reserved, Open)
            | (Reserved, Submitted)
            | (Submitted, PartiallyFilled)
            | (Submitted, Filled)
            | (PartiallyFilled, Reserved)
            | (PartiallyFilled, Filled)
            | (Open, Expired)
            | (PartiallyFilled, Expired)
            | (Open, SoftCancelled)
            | (PartiallyFilled, SoftCancelled)
            | (Open, CancelPending)
            | (PartiallyFilled, CancelPending)
            | (CancelPending, Cancelled)
            | (Open, Cancelled)
            | (PartiallyFilled, Cancelled)
            | (Open, EpochInvalidated)
            | (PartiallyFilled, EpochInvalidated)
            | (Open, Inactive)
            | (PartiallyFilled, Inactive)
            | (Inactive, Open)
    )
}
