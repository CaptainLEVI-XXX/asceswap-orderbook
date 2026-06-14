use asceswap_engine::AsceSwapEngine;
use asceswap_matcher::MatchConfig;
use asceswap_state::OrderState;
use asceswap_storage::{EngineStore, InMemoryEngineStore};
use asceswap_types::{Address, ClaimSide, Order, Side, B256, U256};
use asceswap_validation::{order_digest, order_hash, SignatureDomain};
use k256::ecdsa::SigningKey;

use crate::wire::{encode_b256, encode_bytes, encode_u256};
use crate::{
    spawn_actor_orderbook_api_service_with_capacity, ActorOrderbookApiService, ApiClaimSide,
    ApiError, ApiEventKind, ApiOrder, ApiOrderState, ApiSide, ApiSignatureCheck,
    CancelOrderRequest, DemoMarketMaker, ListEventsRequest, ListReservationsRequest,
    MarketDepthRequest, OrderStatusRequest, OrderbookApiService, ReservationActionRequest,
    SettlementPayloadRequest, SubmitOrderRequest, SubmitOrderResponseOutcome,
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
        post_only: false,
        rest_on_no_match: true,
        reservation_ttl_secs: Some(10),
    }
}

fn signature(byte: u8) -> String {
    format!("0x{}", format!("{byte:02x}").repeat(65))
}

fn signed_submit_request(order: &Order, now: u64, signature_byte: u8) -> SubmitOrderRequest {
    let mut request = submit_request(order, now);
    request.signature_bytes = Some(signature(signature_byte));
    request
}

fn eoa_order(mut order: Order, private_key_byte: u8) -> Order {
    let signing_key = SigningKey::from_bytes((&[private_key_byte; 32]).into()).unwrap();
    order.maker = Address::from_public_key(signing_key.verifying_key());
    order
}

fn eoa_signature_bytes(order: &Order, domain: SignatureDomain, private_key_byte: u8) -> Vec<u8> {
    let signing_key = SigningKey::from_bytes((&[private_key_byte; 32]).into()).unwrap();
    let digest = order_digest(order, domain);
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(digest.as_slice())
        .unwrap();

    let mut signature_bytes = Vec::with_capacity(65);
    signature_bytes.extend_from_slice(&signature.to_bytes());
    signature_bytes.push(27 + u8::from(recovery_id));
    signature_bytes
}

fn eoa_signature(order: &Order, domain: SignatureDomain, private_key_byte: u8) -> String {
    encode_bytes(&eoa_signature_bytes(order, domain, private_key_byte))
}

fn real_signed_submit_request(
    order: &Order,
    domain: SignatureDomain,
    now: u64,
    private_key_byte: u8,
) -> SubmitOrderRequest {
    let mut request = submit_request(order, now);
    request.validation.signature = ApiSignatureCheck::Unchecked;
    request.validation.require_signature = false;
    request.signature_bytes = Some(eoa_signature(order, domain, private_key_byte));
    request
}

fn post_only_submit_request(order: &Order, now: u64) -> SubmitOrderRequest {
    let mut request = submit_request(order, now);
    request.post_only = true;
    request
}

fn service() -> OrderbookApiService<InMemoryEngineStore> {
    OrderbookApiService::new(AsceSwapEngine::default(), InMemoryEngineStore::new())
}

fn demo_market_maker(domain: SignatureDomain, auto_commit: bool) -> DemoMarketMaker {
    DemoMarketMaker::new(
        [9_u8; 32],
        domain,
        U256::from(1),
        100,
        Some(10),
        auto_commit,
    )
    .unwrap()
}

fn actor_service() -> ActorOrderbookApiService<InMemoryEngineStore> {
    ActorOrderbookApiService::new(InMemoryEngineStore::new(), MatchConfig::default(), 8).unwrap()
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
    service
        .submit_order(signed_submit_request(&maker, 100, 1))
        .unwrap();

    let taker_response = service
        .submit_order(signed_submit_request(&taker, 101, 2))
        .unwrap();
    let (reservation_id, settlement) = match taker_response.outcome {
        SubmitOrderResponseOutcome::Matched {
            reservation_id,
            settlement: Some(settlement),
            ..
        } => (reservation_id, settlement),
        other => panic!("expected matched outcome, got {other:?}"),
    };
    assert_eq!(settlement.taker_order, ApiOrder::from(&taker));
    assert_eq!(settlement.taker_signature, signature(2));
    assert_eq!(settlement.maker_orders, vec![ApiOrder::from(&maker)]);
    assert_eq!(settlement.maker_signatures, vec![signature(1)]);
    assert_eq!(settlement.taker_claim_fill_amount, "100");
    assert_eq!(settlement.maker_claim_fill_amounts, vec!["100".to_string()]);

    let fetched_settlement = service
        .settlement_payload(SettlementPayloadRequest {
            reservation_id: reservation_id.clone(),
        })
        .unwrap();
    assert_eq!(fetched_settlement, settlement);

    let tx_hash = encode_b256(B256::repeat_byte(9));
    let submitted = service
        .mark_reservation_submitted(ReservationActionRequest {
            reservation_id: reservation_id.clone(),
            now: 102,
            tx_hash: Some(tx_hash.clone()),
        })
        .unwrap();
    assert_eq!(submitted.tx_hash.as_deref(), Some(tx_hash.as_str()));
    assert!(submitted
        .events
        .iter()
        .any(|event| event.kind == ApiEventKind::ReservationSubmitted
            && event.tx_hash.as_deref() == Some(tx_hash.as_str())));

    let commit = service
        .commit_reservation(ReservationActionRequest {
            reservation_id: reservation_id.clone(),
            now: 103,
            tx_hash: None,
        })
        .unwrap();

    assert!(!commit.events.is_empty());
    let status = service
        .order_status(OrderStatusRequest {
            order_hash: encode_b256(order_hash(&maker)),
        })
        .unwrap();
    assert_eq!(status.state, ApiOrderState::Filled);

    let reservations = service
        .list_reservations(ListReservationsRequest::default())
        .unwrap();
    assert_eq!(
        reservations.reservations[0].tx_hash.as_deref(),
        Some(tx_hash.as_str())
    );

    let events = service
        .list_events(ListEventsRequest {
            from_sequence: Some(0),
            limit: None,
        })
        .unwrap();
    assert!(events
        .events
        .iter()
        .any(|event| event.kind == ApiEventKind::ReservationCommitted
            && event.tx_hash.as_deref() == Some(tx_hash.as_str())));
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
fn post_only_order_returns_clear_outcome_when_it_would_cross() {
    let mut service = service();
    let maker = sell_order(1, 1, 100, 40);
    let crossing_quote = buy_order(2, 2, 100, 50);
    service.submit_order(submit_request(&maker, 100)).unwrap();

    let response = service
        .submit_order(post_only_submit_request(&crossing_quote, 101))
        .unwrap();

    assert_eq!(
        response.outcome,
        SubmitOrderResponseOutcome::PostOnlyWouldCross
    );
    let status = service
        .order_status(OrderStatusRequest {
            order_hash: encode_b256(order_hash(&crossing_quote)),
        })
        .unwrap();
    assert_eq!(status.state, ApiOrderState::Inactive);
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

#[test]
fn product_flow_verifies_signature_persists_matches_commits_and_recovers() {
    let domain = SignatureDomain::new(U256::from(31_337), Address::repeat_byte(17));
    let mut service = service().with_signature_domain(domain);

    let user_order = eoa_order(sell_order(1, 0, 100, 40), 7);
    let user_signature = eoa_signature_bytes(&user_order, domain, 7);
    let user_response = service
        .submit_order(real_signed_submit_request(&user_order, domain, 100, 7))
        .unwrap();

    assert!(matches!(
        user_response.outcome,
        SubmitOrderResponseOutcome::Rested { .. }
    ));
    assert_eq!(service.store().load_events().unwrap().len(), 3);
    let stored_user_order = service.store().load_orders().unwrap();
    assert_eq!(stored_user_order.len(), 1);
    assert_eq!(stored_user_order[0].snapshot.hash, order_hash(&user_order));
    assert_eq!(
        stored_user_order[0].snapshot.signature,
        Some(user_signature.clone())
    );
    assert_eq!(stored_user_order[0].snapshot.state, OrderState::Open);

    let market_maker_order = eoa_order(buy_order(2, 0, 100, 50), 8);
    let market_maker_signature = eoa_signature_bytes(&market_maker_order, domain, 8);
    let market_maker_response = service
        .submit_order(real_signed_submit_request(
            &market_maker_order,
            domain,
            101,
            8,
        ))
        .unwrap();
    let (reservation_id, settlement) = match market_maker_response.outcome {
        SubmitOrderResponseOutcome::Matched {
            reservation_id,
            maker_count: 1,
            settlement: Some(settlement),
            ..
        } => (reservation_id, settlement),
        other => panic!("expected direct market-maker match, got {other:?}"),
    };

    assert_eq!(settlement.taker_order, ApiOrder::from(&market_maker_order));
    assert_eq!(
        settlement.taker_signature,
        encode_bytes(&market_maker_signature)
    );
    assert_eq!(settlement.maker_orders, vec![ApiOrder::from(&user_order)]);
    assert_eq!(
        settlement.maker_signatures,
        vec![encode_bytes(&user_signature)]
    );
    assert_eq!(settlement.taker_claim_fill_amount, "100");
    assert_eq!(settlement.maker_claim_fill_amounts, vec!["100".to_string()]);

    let stored_reservations = service.store().load_reservations().unwrap();
    assert_eq!(stored_reservations.len(), 1);
    assert_eq!(
        stored_reservations[0].reservation.status,
        asceswap_state::ReservationStatus::Reserved
    );

    service
        .mark_reservation_submitted(ReservationActionRequest {
            reservation_id: reservation_id.clone(),
            now: 102,
            tx_hash: None,
        })
        .unwrap();
    service
        .commit_reservation(ReservationActionRequest {
            reservation_id,
            now: 103,
            tx_hash: None,
        })
        .unwrap();

    let user_status = service
        .order_status(OrderStatusRequest {
            order_hash: encode_b256(order_hash(&user_order)),
        })
        .unwrap();
    assert_eq!(user_status.state, ApiOrderState::Filled);
    let market_maker_status = service
        .order_status(OrderStatusRequest {
            order_hash: encode_b256(order_hash(&market_maker_order)),
        })
        .unwrap();
    assert_eq!(market_maker_status.state, ApiOrderState::Filled);

    let (_, store) = service.into_parts();
    let recovered = OrderbookApiService::recover_from_store(store, MatchConfig::default()).unwrap();
    assert_eq!(
        recovered
            .engine()
            .order_record(order_hash(&user_order))
            .unwrap()
            .state(),
        OrderState::Filled
    );
    assert_eq!(
        recovered
            .engine()
            .order_record(order_hash(&market_maker_order))
            .unwrap()
            .signature,
        Some(market_maker_signature)
    );
}

#[test]
fn demo_market_maker_auto_matches_and_mock_commits_resting_user_order() {
    let domain = SignatureDomain::new(U256::from(31_337), Address::repeat_byte(17));
    let mut service = service()
        .with_signature_domain(domain)
        .with_demo_market_maker(demo_market_maker(domain, true));
    let user_order = eoa_order(sell_order(1, 0, 100, 40), 7);

    let response = service
        .submit_order(real_signed_submit_request(&user_order, domain, 100, 7))
        .unwrap();

    assert!(matches!(
        response.outcome,
        SubmitOrderResponseOutcome::Matched { maker_count: 1, .. }
    ));
    assert!(response
        .events
        .iter()
        .any(|event| event.kind == ApiEventKind::ReservationCommitted));

    let status = service
        .order_status(OrderStatusRequest {
            order_hash: encode_b256(order_hash(&user_order)),
        })
        .unwrap();
    assert_eq!(status.state, ApiOrderState::Filled);
    assert!(!status.resting);
}

#[test]
fn demo_market_maker_does_not_take_post_only_order() {
    let domain = SignatureDomain::new(U256::from(31_337), Address::repeat_byte(17));
    let mut service = service()
        .with_signature_domain(domain)
        .with_demo_market_maker(demo_market_maker(domain, true));
    let user_order = eoa_order(sell_order(1, 0, 100, 40), 7);
    let mut request = real_signed_submit_request(&user_order, domain, 100, 7);
    request.post_only = true;

    let response = service.submit_order(request).unwrap();

    assert!(matches!(
        response.outcome,
        SubmitOrderResponseOutcome::Rested { .. }
    ));
    assert!(!response
        .events
        .iter()
        .any(|event| event.kind == ApiEventKind::ReservationCommitted));
    let status = service
        .order_status(OrderStatusRequest {
            order_hash: encode_b256(order_hash(&user_order)),
        })
        .unwrap();
    assert_eq!(status.state, ApiOrderState::Open);
    assert!(status.resting);
}

#[tokio::test]
async fn actor_demo_market_maker_auto_matches_and_mock_commits_resting_user_order() {
    let domain = SignatureDomain::new(U256::from(31_337), Address::repeat_byte(17));
    let mut service = actor_service()
        .with_signature_domain(domain)
        .with_demo_market_maker(demo_market_maker(domain, true));
    let user_order = eoa_order(sell_order(1, 0, 100, 40), 7);

    let response = service
        .submit_order(real_signed_submit_request(&user_order, domain, 100, 7))
        .await
        .unwrap();

    assert!(matches!(
        response.outcome,
        SubmitOrderResponseOutcome::Matched { maker_count: 1, .. }
    ));
    assert!(response
        .events
        .iter()
        .any(|event| event.kind == ApiEventKind::ReservationCommitted));

    let status = service
        .order_status(OrderStatusRequest {
            order_hash: encode_b256(order_hash(&user_order)),
        })
        .await
        .unwrap();
    assert_eq!(status.state, ApiOrderState::Filled);
    assert!(!status.resting);
}

#[tokio::test]
async fn actor_service_rests_order_and_reads_depth() {
    let mut service = actor_service();
    let order = sell_order(1, 1, 100, 40);

    let response = service
        .submit_order(submit_request(&order, 100))
        .await
        .unwrap();

    assert!(matches!(
        response.outcome,
        SubmitOrderResponseOutcome::Rested { .. }
    ));
    assert_eq!(response.events[0].sequence, 0);
    assert_eq!(service.router().market_count(), 1);

    let depth = service
        .market_depth(MarketDepthRequest {
            market_id: encode_b256(market_id()),
            claim: ApiClaimSide::Payoff,
            side: ApiSide::Sell,
        })
        .await
        .unwrap();
    assert_eq!(depth.levels.len(), 1);
    assert_eq!(depth.levels[0].total_claim_amount, "100");
}

#[tokio::test]
async fn actor_service_recovers_and_continues_event_sequence() {
    let mut service = actor_service();
    let first = sell_order(1, 1, 100, 40);
    service
        .submit_order(submit_request(&first, 100))
        .await
        .unwrap();
    let (_, store) = service.into_parts();

    let mut recovered =
        ActorOrderbookApiService::recover_from_store(store, MatchConfig::default(), 8).unwrap();
    let second = sell_order(2, 2, 100, 45);
    let response = recovered
        .submit_order(submit_request(&second, 101))
        .await
        .unwrap();

    assert_eq!(response.events[0].sequence, 3);
}

#[tokio::test]
async fn actor_service_matches_and_commits_reservation() {
    let mut service = actor_service();
    let maker = sell_order(1, 1, 100, 40);
    let taker = buy_order(2, 2, 100, 50);
    service
        .submit_order(signed_submit_request(&maker, 100, 1))
        .await
        .unwrap();

    let taker_response = service
        .submit_order(signed_submit_request(&taker, 101, 2))
        .await
        .unwrap();
    let (reservation_id, settlement) = match taker_response.outcome {
        SubmitOrderResponseOutcome::Matched {
            reservation_id,
            settlement: Some(settlement),
            ..
        } => (reservation_id, settlement),
        other => panic!("expected matched outcome, got {other:?}"),
    };
    assert_eq!(settlement.taker_signature, signature(2));
    assert_eq!(settlement.maker_signatures, vec![signature(1)]);
    assert_eq!(
        service
            .settlement_payload(SettlementPayloadRequest {
                reservation_id: reservation_id.clone(),
            })
            .await
            .unwrap(),
        settlement
    );

    service
        .mark_reservation_submitted(ReservationActionRequest {
            reservation_id: reservation_id.clone(),
            now: 102,
            tx_hash: None,
        })
        .await
        .unwrap();
    service
        .commit_reservation(ReservationActionRequest {
            reservation_id,
            now: 103,
            tx_hash: None,
        })
        .await
        .unwrap();

    let status = service
        .order_status(OrderStatusRequest {
            order_hash: encode_b256(order_hash(&maker)),
        })
        .await
        .unwrap();
    assert_eq!(status.state, ApiOrderState::Filled);
}

#[tokio::test]
async fn actor_service_returns_empty_depth_for_unknown_market() {
    let mut service = actor_service();

    let depth = service
        .market_depth(MarketDepthRequest {
            market_id: encode_b256(B256::repeat_byte(99)),
            claim: ApiClaimSide::Payoff,
            side: ApiSide::Sell,
        })
        .await
        .unwrap();

    assert!(depth.levels.is_empty());
}

#[tokio::test]
async fn actor_api_handle_serializes_service_requests() {
    let handle = spawn_actor_orderbook_api_service_with_capacity(actor_service(), 8).unwrap();
    let order = sell_order(1, 1, 100, 40);

    let response = handle
        .submit_order(submit_request(&order, 100))
        .await
        .unwrap();

    assert!(matches!(
        response.outcome,
        SubmitOrderResponseOutcome::Rested { .. }
    ));
    let status = handle
        .order_status(OrderStatusRequest {
            order_hash: encode_b256(order_hash(&order)),
        })
        .await
        .unwrap();
    assert_eq!(status.state, ApiOrderState::Open);
}

#[test]
fn actor_api_handle_requires_bounded_nonzero_inbox() {
    let error = spawn_actor_orderbook_api_service_with_capacity(actor_service(), 0).unwrap_err();

    assert_eq!(error, ApiError::ServiceInboxCapacityZero);
}
