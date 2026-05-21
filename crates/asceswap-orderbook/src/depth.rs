use asceswap_math::Price;
use asceswap_types::U256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepthLevel {
    pub price: Price,
    pub total_claim_amount: U256,
    pub order_count: usize,
}
