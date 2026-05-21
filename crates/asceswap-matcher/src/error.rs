use asceswap_math::MathError;
use asceswap_types::{MarketId, OrderError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchError {
    InvalidConfig,
    WrongMarket { expected: MarketId, got: MarketId },
    InvalidTaker(OrderError),
    Math(MathError),
}

impl From<MathError> for MatchError {
    fn from(error: MathError) -> Self {
        Self::Math(error)
    }
}
