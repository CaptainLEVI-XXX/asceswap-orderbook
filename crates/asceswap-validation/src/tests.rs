use alloy_primitives::b256;
use asceswap_types::{Address, ClaimSide, Order, OrderError, Side, B256, U256};

use crate::{
    order_hash, order_typehash, validate_order, OrderValidationContext, SignatureCheck,
    ValidationError,
};

fn order() -> Order {
    Order {
        salt: U256::from(1),
        maker: Address::repeat_byte(1),
        market_id: B256::repeat_byte(2),
        claim: ClaimSide::Payoff,
        maker_amount: U256::from(50),
        taker_amount: U256::from(100),
        side: Side::Buy,
        expiration: U256::from(1_000),
        epoch: U256::from(7),
        max_fee_rate_bps: 100,
    }
}

fn context(order: &Order) -> OrderValidationContext {
    OrderValidationContext::new(999)
        .with_expected_order_hash(order_hash(order))
        .with_maker_epoch(order.epoch)
        .with_fee_rate_bps(50)
        .with_signature(SignatureCheck::Valid)
}

#[test]
fn hashes_orders_deterministically() {
    let first = order_hash(&order());
    let second = order_hash(&order());

    assert_eq!(first, second);
    assert_eq!(
        order_typehash(),
        b256!("b4b7b9f4aba15c466c19b02ff5e18bc8899f0ec73f23f8794136daf08d1b5f50")
    );

    let mut changed = order();
    changed.salt = U256::from(2);
    assert_ne!(first, order_hash(&changed));
}

#[test]
fn hashes_match_solidity_abi_encoding() {
    assert_eq!(
        order_hash(&order()),
        b256!("3718556341f8a7a0e20b3149e66da55d91fb8d50c340a28a51bd4054cdf006b5")
    );
}

#[test]
fn validates_acceptance_context() {
    let order = order();
    let validation_context = context(&order).with_required_signature(true);
    let validated = validate_order(&order, &validation_context).unwrap();

    assert_eq!(validated.order_hash, order_hash(&order));
    assert_eq!(validated.filled_claim_amount, U256::ZERO);
    assert_eq!(validated.remaining_claim_amount, U256::from(100));
}

#[test]
fn rejects_basic_order_failure() {
    let mut order = order();
    order.maker = Address::ZERO;

    assert_eq!(
        validate_order(&order, &OrderValidationContext::new(0)),
        Err(ValidationError::BasicOrder(OrderError::ZeroMaker))
    );
}

#[test]
fn rejects_hash_mismatch() {
    let order = order();
    let context = context(&order).with_expected_order_hash(B256::repeat_byte(99));

    assert!(matches!(
        validate_order(&order, &context),
        Err(ValidationError::OrderHashMismatch { .. })
    ));
}

#[test]
fn rejects_contextual_order_failures() {
    let order = order();

    assert_eq!(
        validate_order(
            &order,
            &context(&order).with_signature(SignatureCheck::Invalid)
        ),
        Err(ValidationError::InvalidSignature)
    );
    assert_eq!(
        validate_order(
            &order,
            &context(&order)
                .with_signature(SignatureCheck::Unchecked)
                .with_required_signature(true)
        ),
        Err(ValidationError::MissingSignatureVerification)
    );
    assert!(matches!(
        validate_order(
            &order,
            &OrderValidationContext::new(1_001).with_maker_epoch(order.epoch)
        ),
        Err(ValidationError::Expired { .. })
    ));
    assert_eq!(
        validate_order(&order, &context(&order).with_cancelled(true)),
        Err(ValidationError::Cancelled)
    );
    assert!(matches!(
        validate_order(&order, &context(&order).with_maker_epoch(U256::from(8))),
        Err(ValidationError::EpochMismatch { .. })
    ));
    assert!(matches!(
        validate_order(&order, &context(&order).with_fee_rate_bps(101)),
        Err(ValidationError::FeeRateTooHigh { .. })
    ));
    assert!(matches!(
        validate_order(&order, &context(&order).with_fee_rate_bps(1_001)),
        Err(ValidationError::InvalidExchangeFeeRate { .. })
    ));
}

#[test]
fn treats_expiration_at_now_as_valid() {
    let mut order = order();
    order.expiration = U256::from(1_000);
    let context = context(&order);

    assert!(validate_order(&order, &context).is_ok());
}

#[test]
fn treats_zero_expiration_as_never_expiring() {
    let mut order = order();
    order.expiration = U256::ZERO;
    let context = context(&order);

    assert!(validate_order(&order, &context).is_ok());
}

#[test]
fn rejects_fully_filled_order() {
    let order = order();
    let context = context(&order).with_filled_claim_amount(order.max_claim_amount());

    assert_eq!(
        validate_order(&order, &context),
        Err(ValidationError::NoRemainingClaim)
    );
}
