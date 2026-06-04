use asceswap_types::{Order, U256};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettlementPayload {
    pub taker_order: Order,
    pub taker_signature: Vec<u8>,
    pub maker_orders: Vec<Order>,
    pub maker_signatures: Vec<Vec<u8>>,
    pub taker_claim_fill_amount: U256,
    pub maker_claim_fill_amounts: Vec<U256>,
}
