use asceswap_engine::AsceSwapEngine;
use asceswap_matcher::MatchConfig;
use asceswap_storage::InMemoryEngineStore;
use asceswap_types::{Address, ClaimSide, Order, Side, B256, U256};
use asceswap_validation::{order_hash, SignatureDomain};

use crate::wire::{encode_b256, encode_u256};
use crate::{
    ApiClaimSide, ApiError, ApiOrder, ApiOrderState, ApiSide, ApiSignatureCheck,
    CancelOrderRequest, MarketDepthRequest, OrderStatusRequest, OrderbookApiService,
    ReservationActionRequest, SubmitOrderRequest, SubmitOrderResponseOutcome,
    ValidationContextRequest,
};

fn market_id() -> B256 {
    B256::repeat_byte(3)
}

fn sell_order(salt: u64, maker: u8, claim_amount: u64, collateral_amount: u64) -> Order {
    Order {
        salt: U256::from(salt),
        maker: Address::repeat_byte(maker),
        market_id: market_id(),
        claim: ClaimSide::Payoff,
        maker_amount: U256::from(claim_amount),
        taker_amount: U256::from(collateral_amount),
        side: Side::Sell,
        expiration: U256::ZERO,
        epoch: U256::from(1),
        max_fee_rate_bps: 100,
    }
}

fn buy_order(salt: u64, maker: u8, claim_amount: u64, collateral_amount: u64) -> Order {
    Order {
        salt: U256::from(salt),
        maker: Address::repeat_byte(maker),
        market_id: market_id(),
        claim: ClaimSide::Payoff,
        maker_amount: U256::from(collateral_amount),
        taker_amount: U256::from(claim_amount),
        side: Side::Buy,
        expiration: U256::ZERO,
        epoch: U256::from(1),
        max_fee_rate_bps: 100,
    }
}

fn validation(order: &Order, now: u64) -> ValidationContextRequest {
    ValidationContextRequest {
        now,
        expected_order_hash: Some(encode_b256(order_hash(order))),
        filled_claim_amount: "0".to_string(),
        cancelled: false,
        maker_epoch: encode_u256(order.epoch),
        fee_rate_bps: 0,
        signature: ApiSignatureCheck::Valid,
        require_signature: true,
    }
}

fn submit_request(order: &Order, now: u64) -> SubmitOrderRequest {
    SubmitOrderRequest {
        order: ApiOrder::from(order),
        validation: validation(order, now),
        signature_bytes: None,
        rest_on_no_match: true,
        reservation_ttl_secs: Some(10),
    }
}

fn service() -> OrderbookApiService<InMemoryEngineStore> {
    OrderbookApiService::new(AsceSwapEngine::default(), InMemoryEngineStore::new())
}

#[test]
fn submit_request_round_trips_json_and_rests_order() {
    let mut service = service();
    let order = sell_order(1, 1, 100, 40);
    let request = submit_request(&order, 100);
    let json = serde_json::to_string(&request).unwrap();
    let decoded = serde_json::from_str::<SubmitOrderRequest>(&json).unwrap();

    let response = service.submit_order(decoded).unwrap();

    assert!(matches!(
        response.outcome,
        SubmitOrderResponseOutcome::Rested { .. }
    ));
    assert_eq!(response.events[0].sequence, 0);
    assert_eq!(response.events[1].sequence, 1);
    assert_eq!(response.events[2].sequence, 2);

    let status = service
        .order_status(OrderStatusRequest {
            order_hash: encode_b256(order_hash(&order)),
        })
        .unwrap();
    assert_eq!(status.state, ApiOrderState::Open);
    assert_eq!(status.remaining_claim_amount, "100");

    let depth = service
        .market_depth(MarketDepthRequest {
            market_id: encode_b256(market_id()),
            claim: ApiClaimSide::Payoff,
            side: ApiSide::Sell,
        })
        .unwrap();
    assert_eq!(depth.levels.len(), 1);
    assert_eq!(depth.levels[0].total_claim_amount, "100");
}

#[test]
fn crossed_order_can_be_submitted_and_committed_through_api() {
    let mut service = service();
    let maker = sell_order(1, 1, 100, 40);
    let taker = buy_order(2, 2, 100, 50);
    service.submit_order(submit_request(&maker, 100)).unwrap();

    let taker_response = service.submit_order(submit_request(&taker, 101)).unwrap();
    let reservation_id = match taker_response.outcome {
        SubmitOrderResponseOutcome::Matched { reservation_id, .. } => reservation_id,
        other => panic!("expected matched outcome, got {other:?}"),
    };

    service
        .mark_reservation_submitted(ReservationActionRequest {
            reservation_id: reservation_id.clone(),
            now: 102,
        })
        .unwrap();
    let commit = service
        .commit_reservation(ReservationActionRequest {
            reservation_id,
            now: 103,
        })
        .unwrap();

    assert!(!commit.events.is_empty());
    let status = service
        .order_status(OrderStatusRequest {
            order_hash: encode_b256(order_hash(&maker)),
        })
        .unwrap();
    assert_eq!(status.state, ApiOrderState::Filled);
}

#[test]
fn cancel_order_removes_resting_liquidity() {
    let mut service = service();
    let order = sell_order(1, 1, 100, 40);
    let order_hash = encode_b256(order_hash(&order));
    service.submit_order(submit_request(&order, 100)).unwrap();

    let cancel = service
        .cancel_order(CancelOrderRequest {
            order_hash: order_hash.clone(),
            now: 101,
        })
        .unwrap();

    assert_eq!(cancel.order_hash, order_hash.clone());
    let status = service
        .order_status(OrderStatusRequest { order_hash })
        .unwrap();
    assert_eq!(status.state, ApiOrderState::Cancelled);
    assert!(!status.resting);
}

#[test]
fn recovered_service_continues_event_sequence() {
    let mut service = service();
    let first = sell_order(1, 1, 100, 40);
    service.submit_order(submit_request(&first, 100)).unwrap();
    let (_, store) = service.into_parts();

    let mut recovered =
        OrderbookApiService::recover_from_store(store, MatchConfig::default()).unwrap();
    let second = sell_order(2, 2, 100, 45);
    let response = recovered
        .submit_order(submit_request(&second, 101))
        .unwrap();

    assert_eq!(response.events[0].sequence, 3);
}

#[test]
fn rejects_bad_hex_order_hash() {
    let service = service();
    let error = service
        .order_status(OrderStatusRequest {
            order_hash: "1234".to_string(),
        })
        .unwrap_err();

    assert_eq!(
        error,
        ApiError::InvalidField {
            field: "order_hash",
            reason: "missing 0x prefix",
        }
    );
}

#[test]
fn configured_service_requires_wire_signature_bytes() {
    let mut service = service().with_signature_domain(SignatureDomain::new(
        U256::from(31_337),
        Address::repeat_byte(17),
    ));
    let order = sell_order(1, 1, 100, 40);
    let response = service.submit_order(submit_request(&order, 100)).unwrap();

    assert!(matches!(
        response.outcome,
        SubmitOrderResponseOutcome::Rejected { reason }
            if reason == "MissingSignatureVerification"
    ));
}
