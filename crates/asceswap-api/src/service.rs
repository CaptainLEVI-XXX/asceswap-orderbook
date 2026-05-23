use asceswap_engine::{
    AsceSwapEngine, EngineError, EngineEvent, ReservationUpdateResult,
    SubmitOrderOutcome as EngineSubmitOrderOutcome,
};
use asceswap_matcher::MatchConfig;
use asceswap_math::remaining_claim_amount;
use asceswap_storage::EngineStore;

use crate::event::ApiEvent;
use crate::request::{
    CancelOrderRequest, MarketDepthRequest, OrderStatusRequest, ReservationActionRequest,
    SubmitOrderRequest,
};
use crate::response::{
    CancelOrderResponse, DepthLevelResponse, MarketDepthResponse, OrderStatusResponse,
    ReservationActionResponse, SubmitOrderResponse, SubmitOrderResponseOutcome,
};
use crate::wire::{encode_b256, encode_u256, ApiMatchKind};
use crate::ApiError;

#[derive(Clone, Debug)]
pub struct OrderbookApiService<S> {
    engine: AsceSwapEngine,
    store: S,
    next_event_sequence: u64,
}

impl<S: EngineStore> OrderbookApiService<S> {
    pub fn new(engine: AsceSwapEngine, store: S) -> Self {
        Self {
            engine,
            store,
            next_event_sequence: 0,
        }
    }

    pub fn recover_from_store(store: S, match_config: MatchConfig) -> Result<Self, ApiError> {
        let next_event_sequence = store
            .last_event_sequence()?
            .map(|sequence| sequence.checked_add(1).ok_or(ApiError::SequenceOverflow))
            .transpose()?
            .unwrap_or(0);
        let engine = store.recover_engine(match_config)?;

        Ok(Self {
            engine,
            store,
            next_event_sequence,
        })
    }

    pub fn engine(&self) -> &AsceSwapEngine {
        &self.engine
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn into_parts(self) -> (AsceSwapEngine, S) {
        (self.engine, self.store)
    }

    pub fn submit_order(
        &mut self,
        request: SubmitOrderRequest,
    ) -> Result<SubmitOrderResponse, ApiError> {
        let now = request.validation.now;
        let result = self.engine.submit_order(request.to_command()?)?;
        let events = self.persist_and_project_events(now, &result.events)?;

        let outcome = match result.outcome {
            EngineSubmitOrderOutcome::Rejected { reason } => SubmitOrderResponseOutcome::Rejected {
                reason: format!("{reason:?}"),
            },
            EngineSubmitOrderOutcome::Rested { price } => SubmitOrderResponseOutcome::Rested {
                price_wad: encode_u256(price.wad()),
            },
            EngineSubmitOrderOutcome::Inactive => SubmitOrderResponseOutcome::Inactive,
            EngineSubmitOrderOutcome::Matched {
                reservation_id,
                plan,
            } => SubmitOrderResponseOutcome::Matched {
                reservation_id: encode_b256(reservation_id),
                match_kind: ApiMatchKind::from(plan.match_kind),
                taker_claim_fill_amount: encode_u256(plan.taker_claim_fill_amount),
                maker_count: plan.maker_fills.len(),
            },
        };

        Ok(SubmitOrderResponse {
            order_hash: encode_b256(result.order_hash),
            outcome,
            events,
        })
    }

    pub fn cancel_order(
        &mut self,
        request: CancelOrderRequest,
    ) -> Result<CancelOrderResponse, ApiError> {
        let now = request.now;
        let result = self.engine.cancel_order(request.to_command()?)?;
        let events = self.persist_and_project_events(now, &result.events)?;

        Ok(CancelOrderResponse {
            order_hash: encode_b256(result.order_hash),
            events,
        })
    }

    pub fn mark_reservation_submitted(
        &mut self,
        request: ReservationActionRequest,
    ) -> Result<ReservationActionResponse, ApiError> {
        let reservation_id = request.reservation_id()?;
        let result = self
            .engine
            .mark_reservation_submitted(reservation_id, request.now)?;
        self.reservation_response(request.now, result)
    }

    pub fn release_reservation(
        &mut self,
        request: ReservationActionRequest,
    ) -> Result<ReservationActionResponse, ApiError> {
        let result = self.engine.release_reservation(request.reservation_id()?)?;
        self.reservation_response(request.now, result)
    }

    pub fn expire_reservation(
        &mut self,
        request: ReservationActionRequest,
    ) -> Result<ReservationActionResponse, ApiError> {
        let result = self
            .engine
            .expire_reservation(request.reservation_id()?, request.now)?;
        self.reservation_response(request.now, result)
    }

    pub fn commit_reservation(
        &mut self,
        request: ReservationActionRequest,
    ) -> Result<ReservationActionResponse, ApiError> {
        let result = self.engine.commit_reservation(request.reservation_id()?)?;
        self.reservation_response(request.now, result)
    }

    pub fn order_status(
        &self,
        request: OrderStatusRequest,
    ) -> Result<OrderStatusResponse, ApiError> {
        let order_hash = request.order_hash()?;
        let record = self
            .engine
            .order_record(order_hash)
            .ok_or_else(|| ApiError::OrderNotFound(request.order_hash.clone()))?;
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

    pub fn market_depth(
        &self,
        request: MarketDepthRequest,
    ) -> Result<MarketDepthResponse, ApiError> {
        let market_id = request.market_id()?;
        let levels = if let Some(book) = self.engine.market_book(market_id) {
            book.depth(request.claim.into(), request.side.into())
                .map_err(EngineError::from)?
                .into_iter()
                .map(|level| DepthLevelResponse {
                    price_wad: encode_u256(level.price.wad()),
                    total_claim_amount: encode_u256(level.total_claim_amount),
                    order_count: level.order_count,
                })
                .collect()
        } else {
            Vec::new()
        };

        Ok(MarketDepthResponse {
            market_id: encode_b256(market_id),
            claim: request.claim,
            side: request.side,
            levels,
        })
    }

    fn reservation_response(
        &mut self,
        now: u64,
        result: ReservationUpdateResult,
    ) -> Result<ReservationActionResponse, ApiError> {
        let events = self.persist_and_project_events(now, &result.events)?;
        Ok(ReservationActionResponse {
            reservation_id: encode_b256(result.reservation_id),
            events,
        })
    }

    fn persist_and_project_events(
        &mut self,
        now: u64,
        events: &[EngineEvent],
    ) -> Result<Vec<ApiEvent>, ApiError> {
        let first_sequence = self.next_event_sequence;
        let next_event_sequence = first_sequence
            .checked_add(events.len() as u64)
            .ok_or(ApiError::SequenceOverflow)?;

        self.store
            .append_engine_events(first_sequence, now, events)?;
        self.store
            .persist_engine_snapshot(self.engine.snapshot(), now)?;
        self.next_event_sequence = next_event_sequence;

        Ok(events
            .iter()
            .enumerate()
            .map(|(offset, event)| ApiEvent::from_engine(first_sequence + offset as u64, event))
            .collect())
    }
}
