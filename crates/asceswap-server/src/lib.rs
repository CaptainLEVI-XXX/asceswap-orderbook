use std::sync::Arc;

use asceswap_api::{
    ActorOrderbookApiHandle, ApiClaimSide, ApiError, ApiEvent, ApiSide, CancelOrderRequest,
    ListEventsRequest, ListOrdersRequest, ListReservationsRequest, MarketDepthRequest,
    OrderStatusRequest, OrderbookApiService, ReservationActionRequest, ReservationActionResponse,
    SettlementPayloadRequest, SettlementPayloadResponse, SubmitOrderRequest,
};
use asceswap_storage::EngineStore;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, Mutex};

const EVENT_CHANNEL_CAPACITY: usize = 1_024;

#[derive(Debug, Deserialize, Serialize)]
pub struct HealthResponse {
    pub status: String,
}

pub struct ServerState<S> {
    service: Arc<Mutex<OrderbookApiService<S>>>,
    events: broadcast::Sender<ApiEvent>,
}

#[derive(Clone)]
pub struct ActorServerState {
    service: ActorOrderbookApiHandle,
    events: broadcast::Sender<ApiEvent>,
}

impl<S> Clone for ServerState<S> {
    fn clone(&self) -> Self {
        Self {
            service: Arc::clone(&self.service),
            events: self.events.clone(),
        }
    }
}

impl ActorServerState {
    pub fn new(service: ActorOrderbookApiHandle) -> Self {
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self { service, events }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ApiEvent> {
        self.events.subscribe()
    }
}

impl<S> ServerState<S> {
    pub fn new(service: OrderbookApiService<S>) -> Self {
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            service: Arc::new(Mutex::new(service)),
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ApiEvent> {
        self.events.subscribe()
    }
}

pub fn router<S>(service: OrderbookApiService<S>) -> Router
where
    S: EngineStore + Send + 'static,
{
    router_from_state(ServerState::new(service))
}

pub fn router_from_state<S>(state: ServerState<S>) -> Router
where
    S: EngineStore + Send + 'static,
{
    Router::new()
        .route("/healthz", get(healthz))
        .route("/orders", get(list_orders::<S>).post(submit_order::<S>))
        .route("/orders/cancel", post(cancel_order::<S>))
        .route("/orders/:order_hash", get(order_status::<S>))
        .route("/markets", get(list_markets::<S>))
        .route("/markets/:market_id/orders", get(list_market_orders::<S>))
        .route("/markets/:market_id/depth", get(market_depth::<S>))
        .route("/events", get(list_events::<S>))
        .route("/reservations", get(list_reservations::<S>))
        .route(
            "/reservations/:reservation_id/submitted",
            post(mark_reservation_submitted::<S>),
        )
        .route(
            "/reservations/:reservation_id/release",
            post(release_reservation::<S>),
        )
        .route(
            "/reservations/:reservation_id/expire",
            post(expire_reservation::<S>),
        )
        .route(
            "/reservations/:reservation_id/commit",
            post(commit_reservation::<S>),
        )
        .route(
            "/reservations/:reservation_id/settlement",
            get(settlement_payload::<S>),
        )
        .route("/ws", get(websocket::<S>))
        .with_state(state)
}

pub fn actor_router(service: ActorOrderbookApiHandle) -> Router {
    actor_router_from_state(ActorServerState::new(service))
}

pub fn actor_router_from_state(state: ActorServerState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/orders", get(actor_list_orders).post(actor_submit_order))
        .route("/orders/cancel", post(actor_cancel_order))
        .route("/orders/:order_hash", get(actor_order_status))
        .route("/markets", get(actor_list_markets))
        .route("/markets/:market_id/orders", get(actor_list_market_orders))
        .route("/markets/:market_id/depth", get(actor_market_depth))
        .route("/events", get(actor_list_events))
        .route("/reservations", get(actor_list_reservations))
        .route(
            "/reservations/:reservation_id/submitted",
            post(actor_mark_reservation_submitted),
        )
        .route(
            "/reservations/:reservation_id/release",
            post(actor_release_reservation),
        )
        .route(
            "/reservations/:reservation_id/expire",
            post(actor_expire_reservation),
        )
        .route(
            "/reservations/:reservation_id/commit",
            post(actor_commit_reservation),
        )
        .route(
            "/reservations/:reservation_id/settlement",
            get(actor_settlement_payload),
        )
        .route("/ws", get(actor_websocket))
        .with_state(state)
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}

async fn submit_order<S>(
    State(state): State<ServerState<S>>,
    Json(request): Json<SubmitOrderRequest>,
) -> Result<Json<asceswap_api::SubmitOrderResponse>, ServerError>
where
    S: EngineStore + Send + 'static,
{
    let response = {
        let mut service = state.service.lock().await;
        service.submit_order(request)?
    };
    publish_events(&state, &response.events);
    Ok(Json(response))
}

async fn cancel_order<S>(
    State(state): State<ServerState<S>>,
    Json(request): Json<CancelOrderRequest>,
) -> Result<Json<asceswap_api::CancelOrderResponse>, ServerError>
where
    S: EngineStore + Send + 'static,
{
    let response = {
        let mut service = state.service.lock().await;
        service.cancel_order(request)?
    };
    publish_events(&state, &response.events);
    Ok(Json(response))
}

async fn order_status<S>(
    State(state): State<ServerState<S>>,
    Path(order_hash): Path<String>,
) -> Result<Json<asceswap_api::OrderStatusResponse>, ServerError>
where
    S: EngineStore + Send + 'static,
{
    let service = state.service.lock().await;
    Ok(Json(
        service.order_status(OrderStatusRequest { order_hash })?,
    ))
}

async fn list_orders<S>(
    State(state): State<ServerState<S>>,
    Query(request): Query<ListOrdersRequest>,
) -> Result<Json<asceswap_api::ListOrdersResponse>, ServerError>
where
    S: EngineStore + Send + 'static,
{
    let service = state.service.lock().await;
    Ok(Json(service.list_orders(request)?))
}

async fn list_market_orders<S>(
    State(state): State<ServerState<S>>,
    Path(market_id): Path<String>,
    Query(mut request): Query<ListOrdersRequest>,
) -> Result<Json<asceswap_api::ListOrdersResponse>, ServerError>
where
    S: EngineStore + Send + 'static,
{
    request.market_id = Some(market_id);
    let service = state.service.lock().await;
    Ok(Json(service.list_orders(request)?))
}

async fn list_markets<S>(
    State(state): State<ServerState<S>>,
) -> Result<Json<asceswap_api::ListMarketsResponse>, ServerError>
where
    S: EngineStore + Send + 'static,
{
    let service = state.service.lock().await;
    Ok(Json(service.list_markets()?))
}

async fn list_events<S>(
    State(state): State<ServerState<S>>,
    Query(request): Query<ListEventsRequest>,
) -> Result<Json<asceswap_api::ListEventsResponse>, ServerError>
where
    S: EngineStore + Send + 'static,
{
    let service = state.service.lock().await;
    Ok(Json(service.list_events(request)?))
}

async fn list_reservations<S>(
    State(state): State<ServerState<S>>,
    Query(request): Query<ListReservationsRequest>,
) -> Result<Json<asceswap_api::ListReservationsResponse>, ServerError>
where
    S: EngineStore + Send + 'static,
{
    let service = state.service.lock().await;
    Ok(Json(service.list_reservations(request)?))
}

async fn actor_submit_order(
    State(state): State<ActorServerState>,
    Json(request): Json<SubmitOrderRequest>,
) -> Result<Json<asceswap_api::SubmitOrderResponse>, ServerError> {
    let response = state.service.submit_order(request).await?;
    publish_actor_events(&state, &response.events);
    Ok(Json(response))
}

async fn actor_cancel_order(
    State(state): State<ActorServerState>,
    Json(request): Json<CancelOrderRequest>,
) -> Result<Json<asceswap_api::CancelOrderResponse>, ServerError> {
    let response = state.service.cancel_order(request).await?;
    publish_actor_events(&state, &response.events);
    Ok(Json(response))
}

async fn actor_order_status(
    State(state): State<ActorServerState>,
    Path(order_hash): Path<String>,
) -> Result<Json<asceswap_api::OrderStatusResponse>, ServerError> {
    Ok(Json(
        state
            .service
            .order_status(OrderStatusRequest { order_hash })
            .await?,
    ))
}

async fn actor_list_orders(
    State(state): State<ActorServerState>,
    Query(request): Query<ListOrdersRequest>,
) -> Result<Json<asceswap_api::ListOrdersResponse>, ServerError> {
    Ok(Json(state.service.list_orders(request).await?))
}

async fn actor_list_market_orders(
    State(state): State<ActorServerState>,
    Path(market_id): Path<String>,
    Query(mut request): Query<ListOrdersRequest>,
) -> Result<Json<asceswap_api::ListOrdersResponse>, ServerError> {
    request.market_id = Some(market_id);
    Ok(Json(state.service.list_orders(request).await?))
}

async fn actor_list_markets(
    State(state): State<ActorServerState>,
) -> Result<Json<asceswap_api::ListMarketsResponse>, ServerError> {
    Ok(Json(state.service.list_markets().await?))
}

async fn actor_list_events(
    State(state): State<ActorServerState>,
    Query(request): Query<ListEventsRequest>,
) -> Result<Json<asceswap_api::ListEventsResponse>, ServerError> {
    Ok(Json(state.service.list_events(request).await?))
}

async fn actor_list_reservations(
    State(state): State<ActorServerState>,
    Query(request): Query<ListReservationsRequest>,
) -> Result<Json<asceswap_api::ListReservationsResponse>, ServerError> {
    Ok(Json(state.service.list_reservations(request).await?))
}

async fn actor_market_depth(
    State(state): State<ActorServerState>,
    Path(market_id): Path<String>,
    Query(query): Query<DepthQuery>,
) -> Result<Json<asceswap_api::MarketDepthResponse>, ServerError> {
    Ok(Json(
        state
            .service
            .market_depth(MarketDepthRequest {
                market_id,
                claim: query.claim,
                side: query.side,
            })
            .await?,
    ))
}

async fn actor_settlement_payload(
    State(state): State<ActorServerState>,
    Path(reservation_id): Path<String>,
) -> Result<Json<SettlementPayloadResponse>, ServerError> {
    Ok(Json(
        state
            .service
            .settlement_payload(SettlementPayloadRequest { reservation_id })
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
struct DepthQuery {
    claim: ApiClaimSide,
    side: ApiSide,
}

async fn market_depth<S>(
    State(state): State<ServerState<S>>,
    Path(market_id): Path<String>,
    Query(query): Query<DepthQuery>,
) -> Result<Json<asceswap_api::MarketDepthResponse>, ServerError>
where
    S: EngineStore + Send + 'static,
{
    let service = state.service.lock().await;
    Ok(Json(service.market_depth(MarketDepthRequest {
        market_id,
        claim: query.claim,
        side: query.side,
    })?))
}

async fn settlement_payload<S>(
    State(state): State<ServerState<S>>,
    Path(reservation_id): Path<String>,
) -> Result<Json<SettlementPayloadResponse>, ServerError>
where
    S: EngineStore + Send + 'static,
{
    let service = state.service.lock().await;
    Ok(Json(service.settlement_payload(
        SettlementPayloadRequest { reservation_id },
    )?))
}

#[derive(Debug, Deserialize)]
struct ReservationActionBody {
    now: u64,
    #[serde(default)]
    tx_hash: Option<String>,
}

async fn actor_mark_reservation_submitted(
    State(state): State<ActorServerState>,
    Path(reservation_id): Path<String>,
    Json(body): Json<ReservationActionBody>,
) -> Result<Json<ReservationActionResponse>, ServerError> {
    actor_reservation_action(state, reservation_id, body, |service, request| async move {
        service.mark_reservation_submitted(request).await
    })
    .await
}

async fn actor_release_reservation(
    State(state): State<ActorServerState>,
    Path(reservation_id): Path<String>,
    Json(body): Json<ReservationActionBody>,
) -> Result<Json<ReservationActionResponse>, ServerError> {
    actor_reservation_action(state, reservation_id, body, |service, request| async move {
        service.release_reservation(request).await
    })
    .await
}

async fn actor_expire_reservation(
    State(state): State<ActorServerState>,
    Path(reservation_id): Path<String>,
    Json(body): Json<ReservationActionBody>,
) -> Result<Json<ReservationActionResponse>, ServerError> {
    actor_reservation_action(state, reservation_id, body, |service, request| async move {
        service.expire_reservation(request).await
    })
    .await
}

async fn actor_commit_reservation(
    State(state): State<ActorServerState>,
    Path(reservation_id): Path<String>,
    Json(body): Json<ReservationActionBody>,
) -> Result<Json<ReservationActionResponse>, ServerError> {
    actor_reservation_action(state, reservation_id, body, |service, request| async move {
        service.commit_reservation(request).await
    })
    .await
}

async fn actor_reservation_action<F, Fut>(
    state: ActorServerState,
    reservation_id: String,
    body: ReservationActionBody,
    action: F,
) -> Result<Json<ReservationActionResponse>, ServerError>
where
    F: FnOnce(ActorOrderbookApiHandle, ReservationActionRequest) -> Fut,
    Fut: std::future::Future<Output = Result<ReservationActionResponse, ApiError>>,
{
    let request = ReservationActionRequest {
        reservation_id,
        now: body.now,
        tx_hash: body.tx_hash,
    };
    let response = action(state.service.clone(), request).await?;
    publish_actor_events(&state, &response.events);
    Ok(Json(response))
}

async fn mark_reservation_submitted<S>(
    State(state): State<ServerState<S>>,
    Path(reservation_id): Path<String>,
    Json(body): Json<ReservationActionBody>,
) -> Result<Json<ReservationActionResponse>, ServerError>
where
    S: EngineStore + Send + 'static,
{
    reservation_action(state, reservation_id, body, |service, request| {
        service.mark_reservation_submitted(request)
    })
    .await
}

async fn release_reservation<S>(
    State(state): State<ServerState<S>>,
    Path(reservation_id): Path<String>,
    Json(body): Json<ReservationActionBody>,
) -> Result<Json<ReservationActionResponse>, ServerError>
where
    S: EngineStore + Send + 'static,
{
    reservation_action(state, reservation_id, body, |service, request| {
        service.release_reservation(request)
    })
    .await
}

async fn expire_reservation<S>(
    State(state): State<ServerState<S>>,
    Path(reservation_id): Path<String>,
    Json(body): Json<ReservationActionBody>,
) -> Result<Json<ReservationActionResponse>, ServerError>
where
    S: EngineStore + Send + 'static,
{
    reservation_action(state, reservation_id, body, |service, request| {
        service.expire_reservation(request)
    })
    .await
}

async fn commit_reservation<S>(
    State(state): State<ServerState<S>>,
    Path(reservation_id): Path<String>,
    Json(body): Json<ReservationActionBody>,
) -> Result<Json<ReservationActionResponse>, ServerError>
where
    S: EngineStore + Send + 'static,
{
    reservation_action(state, reservation_id, body, |service, request| {
        service.commit_reservation(request)
    })
    .await
}

async fn reservation_action<S>(
    state: ServerState<S>,
    reservation_id: String,
    body: ReservationActionBody,
    action: impl FnOnce(
        &mut OrderbookApiService<S>,
        ReservationActionRequest,
    ) -> Result<ReservationActionResponse, ApiError>,
) -> Result<Json<ReservationActionResponse>, ServerError>
where
    S: EngineStore + Send + 'static,
{
    let request = ReservationActionRequest {
        reservation_id,
        now: body.now,
        tx_hash: body.tx_hash,
    };
    let response = {
        let mut service = state.service.lock().await;
        action(&mut service, request)?
    };
    publish_events(&state, &response.events);
    Ok(Json(response))
}

async fn actor_websocket(
    State(state): State<ActorServerState>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| websocket_events(socket, state.subscribe()))
}

async fn websocket<S>(
    State(state): State<ServerState<S>>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse
where
    S: EngineStore + Send + 'static,
{
    ws.on_upgrade(move |socket| websocket_events(socket, state.subscribe()))
}

async fn websocket_events(mut socket: WebSocket, mut events: broadcast::Receiver<ApiEvent>) {
    loop {
        match events.recv().await {
            Ok(event) => {
                let Ok(payload) = serde_json::to_string(&event) else {
                    continue;
                };
                if socket.send(Message::Text(payload)).await.is_err() {
                    break;
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }
}

fn publish_events<S>(state: &ServerState<S>, events: &[ApiEvent]) {
    for event in events {
        let _ = state.events.send(event.clone());
    }
}

fn publish_actor_events(state: &ActorServerState, events: &[ApiEvent]) {
    for event in events {
        let _ = state.events.send(event.clone());
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug)]
pub struct ServerError(ApiError);

impl From<ApiError> for ServerError {
    fn from(error: ApiError) -> Self {
        Self(error)
    }
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            ApiError::InvalidField { .. } => StatusCode::BAD_REQUEST,
            ApiError::OrderNotFound(_) => StatusCode::NOT_FOUND,
            ApiError::SequenceOverflow
            | ApiError::ServiceClosed
            | ApiError::ServiceInboxCapacityZero
            | ApiError::Actor(_)
            | ApiError::Engine(_)
            | ApiError::Storage(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Json(ErrorResponse {
            error: format!("{:?}", self.0),
        });
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests;
