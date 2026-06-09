use asceswap_api::{
    spawn_actor_orderbook_api_service_with_capacity, ActorOrderbookApiService, ApiClaimSide,
    ApiOrder, ApiReservationStatus, ApiSide, ApiSignatureCheck, ListEventsResponse,
    ListMarketsResponse, ListOrdersResponse, ListReservationsResponse, OrderbookApiService,
    SubmitOrderResponse, SubmitOrderResponseOutcome, ValidationContextRequest,
};
use asceswap_engine::AsceSwapEngine;
use asceswap_matcher::MatchConfig;
use asceswap_storage::InMemoryEngineStore;
use asceswap_types::{Address, ClaimSide, Order, Side, B256, U256};
use asceswap_validation::order_hash;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde::de::DeserializeOwned;
use tower::ServiceExt;

use crate::{actor_router, actor_router_from_state, router, router_from_state, ActorServerState};
use crate::{HealthResponse, ServerState};

fn market_id() -> B256 {
    B256::repeat_byte(3)
}

fn encode_b256(value: B256) -> String {
    let mut out = String::with_capacity(66);
    out.push_str("0x");
    for byte in value.as_slice() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
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

fn service() -> OrderbookApiService<InMemoryEngineStore> {
    OrderbookApiService::new(AsceSwapEngine::default(), InMemoryEngineStore::new())
}

fn actor_service() -> ActorOrderbookApiService<InMemoryEngineStore> {
    ActorOrderbookApiService::new(InMemoryEngineStore::new(), MatchConfig::default(), 8).unwrap()
}

fn actor_handle() -> asceswap_api::ActorOrderbookApiHandle {
    spawn_actor_orderbook_api_service_with_capacity(actor_service(), 8).unwrap()
}

fn validation(order: &Order, now: u64) -> ValidationContextRequest {
    ValidationContextRequest {
        now,
        expected_order_hash: Some(encode_b256(order_hash(order))),
        filled_claim_amount: "0".to_string(),
        cancelled: false,
        maker_epoch: order.epoch.to_string(),
        fee_rate_bps: 0,
        signature: ApiSignatureCheck::Valid,
        require_signature: true,
    }
}

fn submit_body(order: &Order, now: u64) -> serde_json::Value {
    serde_json::json!({
        "order": ApiOrder::from(order),
        "validation": validation(order, now),
        "post_only": false,
        "rest_on_no_match": true,
        "reservation_ttl_secs": 10,
    })
}

fn signature(byte: u8) -> String {
    format!("0x{}", format!("{byte:02x}").repeat(65))
}

fn signed_submit_body(order: &Order, now: u64, signature_byte: u8) -> serde_json::Value {
    let mut body = submit_body(order, now);
    body["signature_bytes"] = serde_json::Value::String(signature(signature_byte));
    body
}

async fn decode<T: DeserializeOwned>(body: Body) -> T {
    let bytes = to_bytes(body, usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn healthz_returns_ok() {
    let app = router(service());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = decode::<HealthResponse>(response.into_body()).await;
    assert_eq!(body.status, "ok");
}

#[tokio::test]
async fn submit_order_rests_and_broadcasts_events() {
    let state = ServerState::new(service());
    let mut events = state.subscribe();
    let app = router_from_state(state);
    let order = sell_order(1, 1, 100, 40);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/orders")
                .header("content-type", "application/json")
                .body(Body::from(submit_body(&order, 100).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = decode::<SubmitOrderResponse>(response.into_body()).await;
    assert_eq!(body.events.len(), 3);
    assert_eq!(events.recv().await.unwrap().sequence, 0);
}

#[tokio::test]
async fn actor_router_submit_order_rests_and_broadcasts_events() {
    let state = ActorServerState::new(actor_handle());
    let mut events = state.subscribe();
    let app = actor_router_from_state(state);
    let order = sell_order(1, 1, 100, 40);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/orders")
                .header("content-type", "application/json")
                .body(Body::from(submit_body(&order, 100).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = decode::<SubmitOrderResponse>(response.into_body()).await;
    assert_eq!(body.events.len(), 3);
    assert_eq!(events.recv().await.unwrap().sequence, 0);
}

#[tokio::test]
async fn market_depth_reads_resting_liquidity() {
    let app = router(service());
    let order = sell_order(1, 1, 100, 40);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/orders")
                .header("content-type", "application/json")
                .body(Body::from(submit_body(&order, 100).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/markets/{}/depth?claim=payoff&side=sell",
                    encode_b256(market_id())
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = decode::<asceswap_api::MarketDepthResponse>(response.into_body()).await;
    assert_eq!(body.claim, ApiClaimSide::Payoff);
    assert_eq!(body.side, ApiSide::Sell);
    assert_eq!(body.levels.len(), 1);
    assert_eq!(body.levels[0].total_claim_amount, "100");
}

#[tokio::test]
async fn actor_router_market_depth_reads_resting_liquidity() {
    let app = actor_router(actor_handle());
    let order = sell_order(1, 1, 100, 40);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/orders")
                .header("content-type", "application/json")
                .body(Body::from(submit_body(&order, 100).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/markets/{}/depth?claim=payoff&side=sell",
                    encode_b256(market_id())
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = decode::<asceswap_api::MarketDepthResponse>(response.into_body()).await;
    assert_eq!(body.claim, ApiClaimSide::Payoff);
    assert_eq!(body.side, ApiSide::Sell);
    assert_eq!(body.levels.len(), 1);
    assert_eq!(body.levels[0].total_claim_amount, "100");
}

#[tokio::test]
async fn list_orders_filters_user_history_and_market_orders() {
    let app = actor_router(actor_handle());
    let first = sell_order(1, 1, 100, 40);
    let second = sell_order(2, 2, 100, 45);
    for order in [&first, &second] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/orders")
                    .header("content-type", "application/json")
                    .body(Body::from(submit_body(order, 100).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/orders?maker={}&resting=true",
                    ApiOrder::from(&first).maker
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = decode::<ListOrdersResponse>(response.into_body()).await;
    assert_eq!(body.orders.len(), 1);
    assert_eq!(body.orders[0].order, ApiOrder::from(&first));
    assert!(body.orders[0].resting);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/markets/{}/orders?claim=payoff&side=sell&state=open",
                    encode_b256(market_id())
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = decode::<ListOrdersResponse>(response.into_body()).await;
    assert_eq!(body.orders.len(), 2);
    assert!(body
        .orders
        .iter()
        .all(|order| order.state == asceswap_api::ApiOrderState::Open));
}

#[tokio::test]
async fn list_markets_and_events_support_frontend_recovery() {
    let app = actor_router(actor_handle());
    let order = sell_order(1, 1, 100, 40);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/orders")
                .header("content-type", "application/json")
                .body(Body::from(submit_body(&order, 100).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/markets")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = decode::<ListMarketsResponse>(response.into_body()).await;
    assert_eq!(body.markets.len(), 1);
    assert_eq!(body.markets[0].market_id, encode_b256(market_id()));
    assert_eq!(body.markets[0].order_count, 1);
    assert_eq!(body.markets[0].resting_order_count, 1);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/events?from_sequence=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = decode::<ListEventsResponse>(response.into_body()).await;
    assert_eq!(body.events.len(), 2);
    assert_eq!(body.events[0].sequence, 1);
}

#[tokio::test]
async fn list_reservations_filters_by_status_and_market() {
    let app = actor_router(actor_handle());
    let maker = sell_order(1, 1, 100, 40);
    let taker = buy_order(2, 2, 100, 50);
    for (order, signature_byte, now) in [(&maker, 1, 100), (&taker, 2, 101)] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/orders")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        signed_submit_body(order, now, signature_byte).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/reservations?status=reserved&market_id={}",
                    encode_b256(market_id())
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = decode::<ListReservationsResponse>(response.into_body()).await;
    assert_eq!(body.reservations.len(), 1);
    assert_eq!(body.reservations[0].status, ApiReservationStatus::Reserved);
    assert_eq!(body.reservations[0].legs.len(), 2);
}

#[tokio::test]
async fn settlement_payload_route_returns_contract_arguments() {
    let app = actor_router(actor_handle());
    let maker = sell_order(1, 1, 100, 40);
    let taker = buy_order(2, 2, 100, 50);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/orders")
                .header("content-type", "application/json")
                .body(Body::from(signed_submit_body(&maker, 100, 1).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/orders")
                .header("content-type", "application/json")
                .body(Body::from(signed_submit_body(&taker, 101, 2).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = decode::<SubmitOrderResponse>(response.into_body()).await;
    let reservation_id = match body.outcome {
        SubmitOrderResponseOutcome::Matched {
            reservation_id,
            settlement: Some(settlement),
            ..
        } => {
            assert_eq!(settlement.taker_signature, signature(2));
            assert_eq!(settlement.maker_signatures, vec![signature(1)]);
            reservation_id
        }
        other => panic!("expected matched settlement, got {other:?}"),
    };

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/reservations/{reservation_id}/settlement"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = decode::<asceswap_api::SettlementPayloadResponse>(response.into_body()).await;
    assert_eq!(body.taker_order, ApiOrder::from(&taker));
    assert_eq!(body.taker_signature, signature(2));
    assert_eq!(body.maker_orders, vec![ApiOrder::from(&maker)]);
    assert_eq!(body.maker_claim_fill_amounts, vec!["100".to_string()]);
}

#[tokio::test]
async fn bad_order_hash_returns_bad_request() {
    let app = router(service());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/orders/not-a-hash")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn actor_router_bad_order_hash_returns_bad_request() {
    let app = actor_router(actor_handle());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/orders/not-a-hash")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
