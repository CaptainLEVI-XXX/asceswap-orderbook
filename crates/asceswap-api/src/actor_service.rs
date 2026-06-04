use asceswap_engine::{AsceSwapEngine, EngineError, EngineEvent, ReservationUpdateResult};
use asceswap_market_actor::{MarketActorError, MarketActorRouter};
use asceswap_matcher::MatchConfig;
use asceswap_math::remaining_claim_amount;
use asceswap_storage::EngineStore;
use asceswap_validation::SignatureDomain;
use tokio::sync::{mpsc, oneshot};

use crate::request::{
    CancelOrderRequest, MarketDepthRequest, OrderStatusRequest, ReservationActionRequest,
    SettlementPayloadRequest, SubmitOrderRequest,
};
use crate::response::{
    CancelOrderResponse, DepthLevelResponse, MarketDepthResponse, OrderStatusResponse,
    ReservationActionResponse, SettlementPayloadResponse, SubmitOrderResponse,
};
use crate::service::{project_events, settlement_payload_from_engine, submit_outcome_from_engine};
use crate::wire::{encode_b256, encode_u256};
use crate::{ApiError, ApiEvent};

const DEFAULT_API_SERVICE_INBOX_CAPACITY: usize = 1_024;

#[derive(Clone)]
pub struct ActorOrderbookApiHandle {
    sender: mpsc::Sender<ActorOrderbookApiMessage>,
}

impl std::fmt::Debug for ActorOrderbookApiHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ActorOrderbookApiHandle")
            .finish_non_exhaustive()
    }
}

impl ActorOrderbookApiHandle {
    pub async fn submit_order(
        &self,
        request: SubmitOrderRequest,
    ) -> Result<SubmitOrderResponse, ApiError> {
        self.request(|respond_to| ActorOrderbookApiMessage::SubmitOrder {
            request: Box::new(request),
            respond_to,
        })
        .await
    }

    pub async fn cancel_order(
        &self,
        request: CancelOrderRequest,
    ) -> Result<CancelOrderResponse, ApiError> {
        self.request(|respond_to| ActorOrderbookApiMessage::CancelOrder {
            request,
            respond_to,
        })
        .await
    }

    pub async fn mark_reservation_submitted(
        &self,
        request: ReservationActionRequest,
    ) -> Result<ReservationActionResponse, ApiError> {
        self.request(
            |respond_to| ActorOrderbookApiMessage::MarkReservationSubmitted {
                request,
                respond_to,
            },
        )
        .await
    }

    pub async fn release_reservation(
        &self,
        request: ReservationActionRequest,
    ) -> Result<ReservationActionResponse, ApiError> {
        self.request(|respond_to| ActorOrderbookApiMessage::ReleaseReservation {
            request,
            respond_to,
        })
        .await
    }

    pub async fn expire_reservation(
        &self,
        request: ReservationActionRequest,
    ) -> Result<ReservationActionResponse, ApiError> {
        self.request(|respond_to| ActorOrderbookApiMessage::ExpireReservation {
            request,
            respond_to,
        })
        .await
    }

    pub async fn commit_reservation(
        &self,
        request: ReservationActionRequest,
    ) -> Result<ReservationActionResponse, ApiError> {
        self.request(|respond_to| ActorOrderbookApiMessage::CommitReservation {
            request,
            respond_to,
        })
        .await
    }

    pub async fn settlement_payload(
        &self,
        request: SettlementPayloadRequest,
    ) -> Result<SettlementPayloadResponse, ApiError> {
        self.request(|respond_to| ActorOrderbookApiMessage::SettlementPayload {
            request,
            respond_to,
        })
        .await
    }

    pub async fn order_status(
        &self,
        request: OrderStatusRequest,
    ) -> Result<OrderStatusResponse, ApiError> {
        self.request(|respond_to| ActorOrderbookApiMessage::OrderStatus {
            request,
            respond_to,
        })
        .await
    }

    pub async fn market_depth(
        &self,
        request: MarketDepthRequest,
    ) -> Result<MarketDepthResponse, ApiError> {
        self.request(|respond_to| ActorOrderbookApiMessage::MarketDepth {
            request,
            respond_to,
        })
        .await
    }

    async fn request<T>(
        &self,
        build: impl FnOnce(oneshot::Sender<Result<T, ApiError>>) -> ActorOrderbookApiMessage,
    ) -> Result<T, ApiError> {
        let (respond_to, response) = oneshot::channel();
        self.sender
            .send(build(respond_to))
            .await
            .map_err(|_| ApiError::ServiceClosed)?;
        response.await.map_err(|_| ApiError::ServiceClosed)?
    }
}

pub fn spawn_actor_orderbook_api_service<S>(
    service: ActorOrderbookApiService<S>,
) -> ActorOrderbookApiHandle
where
    S: EngineStore + Send + 'static,
{
    spawn_actor_orderbook_api_service_with_capacity(service, DEFAULT_API_SERVICE_INBOX_CAPACITY)
        .expect("default API service inbox capacity must be nonzero")
}

pub fn spawn_actor_orderbook_api_service_with_capacity<S>(
    service: ActorOrderbookApiService<S>,
    inbox_capacity: usize,
) -> Result<ActorOrderbookApiHandle, ApiError>
where
    S: EngineStore + Send + 'static,
{
    if inbox_capacity == 0 {
        return Err(ApiError::ServiceInboxCapacityZero);
    }

    let (sender, receiver) = mpsc::channel(inbox_capacity);
    tokio::spawn(run_actor_orderbook_api_service(service, receiver));

    Ok(ActorOrderbookApiHandle { sender })
}

#[derive(Debug)]
pub struct ActorOrderbookApiService<S> {
    router: MarketActorRouter,
    store: S,
    match_config: MatchConfig,
    inbox_capacity: usize,
    next_event_sequence: u64,
    signature_domain: Option<SignatureDomain>,
}

enum ActorOrderbookApiMessage {
    SubmitOrder {
        request: Box<SubmitOrderRequest>,
        respond_to: oneshot::Sender<Result<SubmitOrderResponse, ApiError>>,
    },
    CancelOrder {
        request: CancelOrderRequest,
        respond_to: oneshot::Sender<Result<CancelOrderResponse, ApiError>>,
    },
    MarkReservationSubmitted {
        request: ReservationActionRequest,
        respond_to: oneshot::Sender<Result<ReservationActionResponse, ApiError>>,
    },
    ReleaseReservation {
        request: ReservationActionRequest,
        respond_to: oneshot::Sender<Result<ReservationActionResponse, ApiError>>,
    },
    ExpireReservation {
        request: ReservationActionRequest,
        respond_to: oneshot::Sender<Result<ReservationActionResponse, ApiError>>,
    },
    CommitReservation {
        request: ReservationActionRequest,
        respond_to: oneshot::Sender<Result<ReservationActionResponse, ApiError>>,
    },
    SettlementPayload {
        request: SettlementPayloadRequest,
        respond_to: oneshot::Sender<Result<SettlementPayloadResponse, ApiError>>,
    },
    OrderStatus {
        request: OrderStatusRequest,
        respond_to: oneshot::Sender<Result<OrderStatusResponse, ApiError>>,
    },
    MarketDepth {
        request: MarketDepthRequest,
        respond_to: oneshot::Sender<Result<MarketDepthResponse, ApiError>>,
    },
}

async fn run_actor_orderbook_api_service<S>(
    mut service: ActorOrderbookApiService<S>,
    mut receiver: mpsc::Receiver<ActorOrderbookApiMessage>,
) where
    S: EngineStore,
{
    while let Some(message) = receiver.recv().await {
        match message {
            ActorOrderbookApiMessage::SubmitOrder {
                request,
                respond_to,
            } => send_response(respond_to, service.submit_order(*request).await),
            ActorOrderbookApiMessage::CancelOrder {
                request,
                respond_to,
            } => send_response(respond_to, service.cancel_order(request).await),
            ActorOrderbookApiMessage::MarkReservationSubmitted {
                request,
                respond_to,
            } => send_response(
                respond_to,
                service.mark_reservation_submitted(request).await,
            ),
            ActorOrderbookApiMessage::ReleaseReservation {
                request,
                respond_to,
            } => send_response(respond_to, service.release_reservation(request).await),
            ActorOrderbookApiMessage::ExpireReservation {
                request,
                respond_to,
            } => send_response(respond_to, service.expire_reservation(request).await),
            ActorOrderbookApiMessage::CommitReservation {
                request,
                respond_to,
            } => send_response(respond_to, service.commit_reservation(request).await),
            ActorOrderbookApiMessage::SettlementPayload {
                request,
                respond_to,
            } => send_response(respond_to, service.settlement_payload(request).await),
            ActorOrderbookApiMessage::OrderStatus {
                request,
                respond_to,
            } => send_response(respond_to, service.order_status(request).await),
            ActorOrderbookApiMessage::MarketDepth {
                request,
                respond_to,
            } => send_response(respond_to, service.market_depth(request).await),
        }
    }
}

fn send_response<T>(respond_to: oneshot::Sender<Result<T, ApiError>>, result: Result<T, ApiError>) {
    let _ = respond_to.send(result);
}

impl<S: EngineStore> ActorOrderbookApiService<S> {
    pub fn new(
        store: S,
        match_config: MatchConfig,
        inbox_capacity: usize,
    ) -> Result<Self, ApiError> {
        if inbox_capacity == 0 {
            return Err(MarketActorError::InboxCapacityZero.into());
        }

        Ok(Self {
            router: MarketActorRouter::new(),
            store,
            match_config,
            inbox_capacity,
            next_event_sequence: 0,
            signature_domain: None,
        })
    }

    pub fn recover_from_store(
        store: S,
        match_config: MatchConfig,
        inbox_capacity: usize,
    ) -> Result<Self, ApiError> {
        if inbox_capacity == 0 {
            return Err(MarketActorError::InboxCapacityZero.into());
        }

        let next_event_sequence = store
            .last_event_sequence()?
            .map(|sequence| sequence.checked_add(1).ok_or(ApiError::SequenceOverflow))
            .transpose()?
            .unwrap_or(0);
        let mut router = MarketActorRouter::new();
        if let Some(snapshot) = store.load_snapshot()? {
            router.spawn_from_snapshot(snapshot.engine, match_config, inbox_capacity)?;
        }

        Ok(Self {
            router,
            store,
            match_config,
            inbox_capacity,
            next_event_sequence,
            signature_domain: None,
        })
    }

    pub fn with_signature_domain(mut self, signature_domain: SignatureDomain) -> Self {
        self.signature_domain = Some(signature_domain);
        self
    }

    pub fn router(&self) -> &MarketActorRouter {
        &self.router
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn into_parts(self) -> (MarketActorRouter, S) {
        (self.router, self.store)
    }

    pub async fn submit_order(
        &mut self,
        request: SubmitOrderRequest,
    ) -> Result<SubmitOrderResponse, ApiError> {
        let now = request.validation.now;
        let command = request.to_command_with_signature_domain(self.signature_domain)?;
        let market_id = command.order.market_id;
        self.ensure_market(market_id)?;

        let result = self.router.submit_order(command).await?;
        let events = self.persist_and_project_events(now, &result.events).await?;

        Ok(SubmitOrderResponse {
            order_hash: encode_b256(result.order_hash),
            outcome: submit_outcome_from_engine(result.outcome),
            events,
        })
    }

    pub async fn cancel_order(
        &mut self,
        request: CancelOrderRequest,
    ) -> Result<CancelOrderResponse, ApiError> {
        let now = request.now;
        let command = request.to_command()?;
        let result = self.router.cancel_order(command).await?;
        let events = self.persist_and_project_events(now, &result.events).await?;

        Ok(CancelOrderResponse {
            order_hash: encode_b256(result.order_hash),
            events,
        })
    }

    pub async fn mark_reservation_submitted(
        &mut self,
        request: ReservationActionRequest,
    ) -> Result<ReservationActionResponse, ApiError> {
        let reservation_id = request.reservation_id()?;
        let result = self
            .router
            .mark_reservation_submitted(reservation_id, request.now)
            .await?;
        self.reservation_response(request.now, result).await
    }

    pub async fn release_reservation(
        &mut self,
        request: ReservationActionRequest,
    ) -> Result<ReservationActionResponse, ApiError> {
        let result = self
            .router
            .release_reservation(request.reservation_id()?)
            .await?;
        self.reservation_response(request.now, result).await
    }

    pub async fn expire_reservation(
        &mut self,
        request: ReservationActionRequest,
    ) -> Result<ReservationActionResponse, ApiError> {
        let result = self
            .router
            .expire_reservation(request.reservation_id()?, request.now)
            .await?;
        self.reservation_response(request.now, result).await
    }

    pub async fn commit_reservation(
        &mut self,
        request: ReservationActionRequest,
    ) -> Result<ReservationActionResponse, ApiError> {
        let result = self
            .router
            .commit_reservation(request.reservation_id()?)
            .await?;
        self.reservation_response(request.now, result).await
    }

    pub async fn settlement_payload(
        &mut self,
        request: SettlementPayloadRequest,
    ) -> Result<SettlementPayloadResponse, ApiError> {
        Ok(settlement_payload_from_engine(
            self.router
                .settlement_payload(request.reservation_id()?)
                .await?,
        ))
    }

    pub async fn order_status(
        &mut self,
        request: OrderStatusRequest,
    ) -> Result<OrderStatusResponse, ApiError> {
        let order_hash = request.order_hash()?;
        let record = match self.router.order_record(order_hash).await {
            Ok(Some(record)) => record,
            Ok(None) | Err(MarketActorError::MissingOrderRoute(_)) => {
                return Err(ApiError::OrderNotFound(request.order_hash));
            }
            Err(error) => return Err(error.into()),
        };
        let remaining = remaining_claim_amount(&record.order, record.filled_claim_amount)
            .map_err(EngineError::from)?;

        Ok(OrderStatusResponse {
            order_hash: encode_b256(order_hash),
            state: record.state().into(),
            filled_claim_amount: encode_u256(record.filled_claim_amount),
            remaining_claim_amount: encode_u256(remaining),
            resting: record.resting,
        })
    }

    pub async fn market_depth(
        &mut self,
        request: MarketDepthRequest,
    ) -> Result<MarketDepthResponse, ApiError> {
        let market_id = request.market_id()?;
        let levels = if self.router.contains_market(market_id) {
            self.router
                .depth(market_id, request.claim.into(), request.side.into())
                .await?
        } else {
            Vec::new()
        };

        Ok(MarketDepthResponse {
            market_id: encode_b256(market_id),
            claim: request.claim,
            side: request.side,
            levels: levels
                .into_iter()
                .map(|level| DepthLevelResponse {
                    price_wad: encode_u256(level.price.wad()),
                    total_claim_amount: encode_u256(level.total_claim_amount),
                    order_count: level.order_count,
                })
                .collect(),
        })
    }

    fn ensure_market(&mut self, market_id: asceswap_types::MarketId) -> Result<(), ApiError> {
        if !self.router.contains_market(market_id) {
            self.router.spawn_market(
                market_id,
                AsceSwapEngine::new(self.match_config),
                self.inbox_capacity,
            )?;
        }

        Ok(())
    }

    async fn reservation_response(
        &mut self,
        now: u64,
        result: ReservationUpdateResult,
    ) -> Result<ReservationActionResponse, ApiError> {
        let events = self.persist_and_project_events(now, &result.events).await?;
        Ok(ReservationActionResponse {
            reservation_id: encode_b256(result.reservation_id),
            events,
        })
    }

    async fn persist_and_project_events(
        &mut self,
        now: u64,
        events: &[EngineEvent],
    ) -> Result<Vec<ApiEvent>, ApiError> {
        let first_sequence = self.next_event_sequence;
        let next_event_sequence = first_sequence
            .checked_add(events.len() as u64)
            .ok_or(ApiError::SequenceOverflow)?;
        let snapshot = self.router.snapshot_all().await?;

        self.store
            .persist_engine_update(first_sequence, now, events, snapshot)?;
        self.next_event_sequence = next_event_sequence;

        Ok(project_events(first_sequence, events))
    }
}
