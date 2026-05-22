use asceswap_matcher::MatchError;
use asceswap_math::MathError;
use asceswap_orderbook::BookError;
use asceswap_state::{OrderState, ReservationId, StateError};
use asceswap_types::{MarketId, OrderHash};
use asceswap_validation::ValidationError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineError {
    DuplicateOrder(OrderHash),
    MissingOrder(OrderHash),
    MissingMarket(MarketId),
    TimeOverflow,
    InvalidOrderState {
        order_hash: OrderHash,
        state: OrderState,
    },
    ReservationExpired(ReservationId),
    Validation(ValidationError),
    Math(MathError),
    Match(MatchError),
    Book(BookError),
    State(StateError),
}

impl From<ValidationError> for EngineError {
    fn from(error: ValidationError) -> Self {
        Self::Validation(error)
    }
}

impl From<MathError> for EngineError {
    fn from(error: MathError) -> Self {
        Self::Math(error)
    }
}

impl From<MatchError> for EngineError {
    fn from(error: MatchError) -> Self {
        Self::Match(error)
    }
}

impl From<BookError> for EngineError {
    fn from(error: BookError) -> Self {
        Self::Book(error)
    }
}

impl From<StateError> for EngineError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}
