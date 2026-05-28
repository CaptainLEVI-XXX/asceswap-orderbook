mod context;
mod error;
mod hash;
mod validator;

pub use context::{OrderValidationContext, SignatureCheck};
pub use error::ValidationError;
pub use hash::{
    domain_separator, domain_typehash, order_digest, order_hash, order_typehash,
    verify_order_eoa_signature, SignatureDomain,
};
pub use validator::{validate_order, ValidatedOrder, MAX_EXCHANGE_FEE_RATE_BPS};

#[cfg(test)]
mod tests;
