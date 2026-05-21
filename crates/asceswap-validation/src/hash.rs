use alloy_primitives::keccak256;
use asceswap_types::{Address, ClaimSide, Order, Side, B256, U256};

const ORDER_TYPE: &str = "Order(uint256 salt,address maker,bytes32 marketId,uint8 claim,uint256 makerAmount,uint256 takerAmount,uint8 side,uint256 expiration,uint256 epoch,uint16 maxFeeRateBps)";

pub fn order_typehash() -> B256 {
    keccak256(ORDER_TYPE.as_bytes())
}

pub fn order_hash(order: &Order) -> B256 {
    let mut encoded = Vec::with_capacity(32 * 11);
    push_b256(&mut encoded, order_typehash());
    push_u256(&mut encoded, order.salt);
    push_address(&mut encoded, order.maker);
    push_b256(&mut encoded, order.market_id);
    push_u256(&mut encoded, U256::from(claim_side_word(order.claim)));
    push_u256(&mut encoded, order.maker_amount);
    push_u256(&mut encoded, order.taker_amount);
    push_u256(&mut encoded, U256::from(side_word(order.side)));
    push_u256(&mut encoded, order.expiration);
    push_u256(&mut encoded, order.epoch);
    push_u256(&mut encoded, U256::from(order.max_fee_rate_bps));

    keccak256(encoded)
}

fn push_b256(encoded: &mut Vec<u8>, value: B256) {
    encoded.extend_from_slice(value.as_slice());
}

fn push_address(encoded: &mut Vec<u8>, value: Address) {
    encoded.extend_from_slice(&[0_u8; 12]);
    encoded.extend_from_slice(value.as_slice());
}

fn push_u256(encoded: &mut Vec<u8>, value: U256) {
    encoded.extend_from_slice(&value.to_be_bytes::<32>());
}

fn claim_side_word(claim: ClaimSide) -> u8 {
    match claim {
        ClaimSide::Residual => 0,
        ClaimSide::Payoff => 1,
    }
}

fn side_word(side: Side) -> u8 {
    match side {
        Side::Buy => 0,
        Side::Sell => 1,
    }
}
