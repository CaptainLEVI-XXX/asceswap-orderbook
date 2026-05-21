pub const CONTRACT_MAX_MAKER_ORDERS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchConfig {
    pub max_maker_orders: usize,
}

impl Default for MatchConfig {
    fn default() -> Self {
        Self {
            max_maker_orders: CONTRACT_MAX_MAKER_ORDERS,
        }
    }
}
