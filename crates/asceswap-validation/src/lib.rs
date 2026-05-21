mod context;
mod error;
mod hash;
mod validator;

pub use context::{OrderValidationContext, SignatureCheck};
pub use error::ValidationError;
pub use hash::{order_hash, order_typehash};
pub use validator::{validate_order, ValidatedOrder, MAX_EXCHANGE_FEE_RATE_BPS};

#[cfg(test)]
mod tests;
