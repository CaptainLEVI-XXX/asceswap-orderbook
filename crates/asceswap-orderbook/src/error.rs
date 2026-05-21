use asceswap_math::MathError;
use asceswap_types::{MarketId, OrderError, OrderHash};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookError {
    WrongMarket { expected: MarketId, got: MarketId },
    DuplicateOrder(OrderHash),
    MissingOrder(OrderHash),
    FilledOrder(OrderHash),
    InvalidOrder(OrderError),
    Math(MathError),
    SequenceOverflow,
    ArithmeticOverflow,
}

impl From<OrderError> for BookError {
    fn from(error: OrderError) -> Self {
        Self::InvalidOrder(error)
    }
}

impl From<MathError> for BookError {
    fn from(error: MathError) -> Self {
        Self::Math(error)
    }
}
