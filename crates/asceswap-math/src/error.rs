use asceswap_types::OrderError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathError {
    Order(OrderError),
    DivisionByZero,
    ZeroFill,
    Overfill,
    ArithmeticOverflow,
    InvalidFeeConfig,
}

impl From<OrderError> for MathError {
    fn from(error: OrderError) -> Self {
        Self::Order(error)
    }
}
