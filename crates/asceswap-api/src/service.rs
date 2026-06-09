use std::collections::{BTreeMap, HashMap};

use asceswap_engine::{
    AsceSwapEngine, EngineError, EngineEvent, ReservationUpdateResult, SettlementPayload,
    SubmitOrder, SubmitOrderOutcome as EngineSubmitOrderOutcome,
};
use asceswap_matcher::MatchConfig;
use asceswap_math::remaining_claim_amount;
use asceswap_state::{ReservationId, ReservationStatus};
use asceswap_storage::{EngineStore, StoredOrder, StoredReservation};
use asceswap_validation::SignatureDomain;

use crate::demo_market_maker::DemoMarketMaker;
use crate::event::ApiEvent;
use crate::request::{
    CancelOrderRequest, ListEventsRequest, ListMarketOrdersRequest, ListOrdersRequest,
    ListReservationsRequest, MarketDepthRequest, OrderStatusRequest, ReservationActionRequest,
    SettlementPayloadRequest, SubmitOrderRequest,
};
use crate::response::{
    CancelOrderResponse, DepthLevelResponse, ListEventsResponse, ListMarketsResponse,
    ListOrdersResponse, ListReservationsResponse, MarketDepthResponse, MarketSummaryResponse,
    OrderStatusResponse, OrderSummaryResponse, ReservationActionResponse, ReservationLegResponse,
    ReservationSummaryResponse, SettlementPayloadResponse, SubmitOrderResponse,
    SubmitOrderResponseOutcome,
};
use crate::wire::{
    encode_b256, encode_bytes, encode_u256, ApiMatchKind, ApiOrder, ApiReservationLegRole,
};
use crate::ApiError;

#[derive(Clone, Debug)]
pub struct OrderbookApiService<S> {
    engine: AsceSwapEngine,
    store: S,
    next_event_sequence: u64,
    signature_domain: Option<SignatureDomain>,
    demo_market_maker: Option<DemoMarketMaker>,
}

impl<S: EngineStore> OrderbookApiService<S> {
    pub fn new(engine: AsceSwapEngine, store: S) -> Self {
        Self {
            engine,
            store,
            next_event_sequence: 0,
            signature_domain: None,
            demo_market_maker: None,
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
            signature_domain: None,
            demo_market_maker: None,
        })
    }

    pub fn with_signature_domain(mut self, signature_domain: SignatureDomain) -> Self {
        self.signature_domain = Some(signature_domain);
        self
    }

    pub fn with_demo_market_maker(mut self, mut demo_market_maker: DemoMarketMaker) -> Self {
        demo_market_maker.ensure_next_salt_at_least(self.next_event_sequence.saturating_add(1));
        self.demo_market_maker = Some(demo_market_maker);
        self
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
        let command = request.to_command_with_signature_domain(self.signature_domain)?;
        let result = self.engine.submit_order(command.clone())?;
        let mut engine_events = result.events.clone();
        let mut outcome = result.outcome.clone();

        if let Some((auto_outcome, mut auto_events)) =
            self.run_demo_market_maker_after_submit(now, &command, &result.outcome)?
        {
            if matches!(auto_outcome, EngineSubmitOrderOutcome::Matched { .. }) {
                outcome = auto_outcome;
            }
            engine_events.append(&mut auto_events);
        }

        let events = self.persist_and_project_events(now, &engine_events)?;

        Ok(SubmitOrderResponse {
            order_hash: encode_b256(result.order_hash),
            outcome: submit_outcome_from_engine(outcome),
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

    pub fn settlement_payload(
        &self,
        request: SettlementPayloadRequest,
    ) -> Result<SettlementPayloadResponse, ApiError> {
        Ok(settlement_payload_from_engine(
            self.engine.settlement_payload(request.reservation_id()?)?,
        ))
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

    pub fn list_orders(&self, request: ListOrdersRequest) -> Result<ListOrdersResponse, ApiError> {
        list_orders_from_store(&self.store, request)
    }

    pub fn list_market_orders(
        &self,
        request: ListMarketOrdersRequest,
    ) -> Result<ListOrdersResponse, ApiError> {
        list_orders_from_store(&self.store, request.to_list_orders_request())
    }

    pub fn list_markets(&self) -> Result<ListMarketsResponse, ApiError> {
        list_markets_from_store(&self.store)
    }

    pub fn list_events(&self, request: ListEventsRequest) -> Result<ListEventsResponse, ApiError> {
        list_events_from_store(&self.store, request)
    }

    pub fn list_reservations(
        &self,
        request: ListReservationsRequest,
    ) -> Result<ListReservationsResponse, ApiError> {
        list_reservations_from_store(&self.store, request)
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

    fn run_demo_market_maker_after_submit(
        &mut self,
        now: u64,
        trigger_command: &SubmitOrder,
        trigger_outcome: &EngineSubmitOrderOutcome,
    ) -> Result<Option<(EngineSubmitOrderOutcome, Vec<EngineEvent>)>, ApiError> {
        if !matches!(trigger_outcome, EngineSubmitOrderOutcome::Rested { .. }) {
            return Ok(None);
        }
        if trigger_command.post_only {
            return Ok(None);
        }

        let Some(demo_market_maker) = self.demo_market_maker.as_mut() else {
            return Ok(None);
        };
        if trigger_command.order.maker == demo_market_maker.maker() {
            return Ok(None);
        }

        let market_maker_command = demo_market_maker.counter_order_for(
            &trigger_command.order,
            trigger_command.validation.filled_claim_amount,
            now,
        )?;
        let auto_commit = demo_market_maker.auto_commit();
        let result = self.engine.submit_order(market_maker_command)?;
        let mut events = result.events.clone();
        if auto_commit {
            if let EngineSubmitOrderOutcome::Matched { reservation_id, .. } = &result.outcome {
                self.append_mock_commit_events(*reservation_id, now, &mut events)?;
            }
        }

        Ok(Some((result.outcome, events)))
    }

    fn append_mock_commit_events(
        &mut self,
        reservation_id: ReservationId,
        now: u64,
        events: &mut Vec<EngineEvent>,
    ) -> Result<(), ApiError> {
        let submitted = self
            .engine
            .mark_reservation_submitted(reservation_id, now)?;
        events.extend(submitted.events);
        let committed = self.engine.commit_reservation(reservation_id)?;
        events.extend(committed.events);
        Ok(())
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
            .persist_engine_update(first_sequence, now, events, self.engine.snapshot())?;
        self.next_event_sequence = next_event_sequence;

        Ok(project_events(first_sequence, events))
    }
}

pub(crate) fn submit_outcome_from_engine(
    outcome: EngineSubmitOrderOutcome,
) -> SubmitOrderResponseOutcome {
    match outcome {
        EngineSubmitOrderOutcome::Rejected { reason } => SubmitOrderResponseOutcome::Rejected {
            reason: format!("{reason:?}"),
        },
        EngineSubmitOrderOutcome::Rested { price } => SubmitOrderResponseOutcome::Rested {
            price_wad: encode_u256(price.wad()),
        },
        EngineSubmitOrderOutcome::PostOnlyWouldCross => {
            SubmitOrderResponseOutcome::PostOnlyWouldCross
        }
        EngineSubmitOrderOutcome::Inactive => SubmitOrderResponseOutcome::Inactive,
        EngineSubmitOrderOutcome::Matched {
            reservation_id,
            plan,
            settlement,
        } => SubmitOrderResponseOutcome::Matched {
            reservation_id: encode_b256(reservation_id),
            match_kind: ApiMatchKind::from(plan.match_kind),
            taker_claim_fill_amount: encode_u256(plan.taker_claim_fill_amount),
            maker_count: plan.maker_fills.len(),
            settlement: settlement.map(settlement_payload_from_engine),
        },
    }
}

pub(crate) fn settlement_payload_from_engine(
    payload: SettlementPayload,
) -> SettlementPayloadResponse {
    SettlementPayloadResponse {
        taker_order: ApiOrder::from(&payload.taker_order),
        taker_signature: encode_bytes(&payload.taker_signature),
        maker_orders: payload.maker_orders.iter().map(ApiOrder::from).collect(),
        maker_signatures: payload
            .maker_signatures
            .iter()
            .map(|signature| encode_bytes(signature))
            .collect(),
        taker_claim_fill_amount: encode_u256(payload.taker_claim_fill_amount),
        maker_claim_fill_amounts: payload
            .maker_claim_fill_amounts
            .into_iter()
            .map(encode_u256)
            .collect(),
    }
}

pub(crate) fn list_orders_from_store<S: EngineStore>(
    store: &S,
    request: ListOrdersRequest,
) -> Result<ListOrdersResponse, ApiError> {
    let maker = request.maker()?;
    let market_id = request.market_id()?;
    let limit = request.limit()?;
    let state = request.state.map(Into::into);
    let claim = request.claim.map(Into::into);
    let side = request.side.map(Into::into);

    let mut orders = store
        .load_orders()?
        .into_iter()
        .filter(|stored| {
            let snapshot = &stored.snapshot;
            maker.map_or(true, |maker| snapshot.order.maker == maker)
                && market_id.map_or(true, |market_id| snapshot.order.market_id == market_id)
                && claim.map_or(true, |claim| snapshot.order.claim == claim)
                && side.map_or(true, |side| snapshot.order.side == side)
                && state.map_or(true, |state| snapshot.state == state)
                && request
                    .resting
                    .map_or(true, |resting| snapshot.resting == resting)
        })
        .collect::<Vec<_>>();
    sort_orders_for_listing(&mut orders);

    let orders = orders
        .into_iter()
        .take(limit)
        .map(|order| order_summary_from_stored(&order))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(ListOrdersResponse { orders })
}

pub(crate) fn list_markets_from_store<S: EngineStore>(
    store: &S,
) -> Result<ListMarketsResponse, ApiError> {
    let mut markets = BTreeMap::<String, MarketSummaryResponse>::new();
    for order in store.load_orders()? {
        let market_id = encode_b256(order.snapshot.order.market_id);
        let entry = markets
            .entry(market_id.clone())
            .or_insert_with(|| MarketSummaryResponse {
                market_id,
                order_count: 0,
                resting_order_count: 0,
            });
        entry.order_count += 1;
        if order.snapshot.resting {
            entry.resting_order_count += 1;
        }
    }

    Ok(ListMarketsResponse {
        markets: markets.into_values().collect(),
    })
}

pub(crate) fn list_events_from_store<S: EngineStore>(
    store: &S,
    request: ListEventsRequest,
) -> Result<ListEventsResponse, ApiError> {
    let from_sequence = request.from_sequence();
    let limit = request.limit()?;
    let events = store
        .load_events()?
        .into_iter()
        .filter(|event| event.sequence >= from_sequence)
        .take(limit)
        .map(|event| ApiEvent::from_engine(event.sequence, &event.event))
        .collect();

    Ok(ListEventsResponse { events })
}

pub(crate) fn list_reservations_from_store<S: EngineStore>(
    store: &S,
    request: ListReservationsRequest,
) -> Result<ListReservationsResponse, ApiError> {
    let status = request.status.map(ReservationStatus::from);
    let market_id = request.market_id()?;
    let order_hash = request.order_hash()?;
    let limit = request.limit()?;
    let order_markets = store
        .load_orders()?
        .into_iter()
        .map(|order| (order.snapshot.hash, order.snapshot.order.market_id))
        .collect::<HashMap<_, _>>();

    let mut reservations = store
        .load_reservations()?
        .into_iter()
        .filter(|stored| {
            let reservation = &stored.reservation;
            status.map_or(true, |status| reservation.status == status)
                && order_hash.map_or(true, |order_hash| {
                    reservation
                        .legs
                        .iter()
                        .any(|leg| leg.order_hash == order_hash)
                })
                && market_id.map_or(true, |market_id| {
                    reservation.legs.iter().any(|leg| {
                        order_markets
                            .get(&leg.order_hash)
                            .map_or(false, |leg_market| *leg_market == market_id)
                    })
                })
        })
        .collect::<Vec<_>>();
    sort_reservations_for_listing(&mut reservations);

    let reservations = reservations
        .into_iter()
        .take(limit)
        .map(reservation_summary_from_stored)
        .collect();

    Ok(ListReservationsResponse { reservations })
}

fn sort_orders_for_listing(orders: &mut [StoredOrder]) {
    orders.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| {
                right
                    .snapshot
                    .accepted_sequence
                    .cmp(&left.snapshot.accepted_sequence)
            })
            .then_with(|| {
                left.snapshot
                    .hash
                    .as_slice()
                    .cmp(right.snapshot.hash.as_slice())
            })
    });
}

fn order_summary_from_stored(order: &StoredOrder) -> Result<OrderSummaryResponse, ApiError> {
    let snapshot = &order.snapshot;
    let remaining = remaining_claim_amount(&snapshot.order, snapshot.filled_claim_amount)
        .map_err(EngineError::from)?;

    Ok(OrderSummaryResponse {
        order_hash: encode_b256(snapshot.hash),
        order: ApiOrder::from(&snapshot.order),
        state: snapshot.state.into(),
        filled_claim_amount: encode_u256(snapshot.filled_claim_amount),
        remaining_claim_amount: encode_u256(remaining),
        resting: snapshot.resting,
        accepted_sequence: snapshot.accepted_sequence,
        created_at: order.created_at,
        updated_at: order.updated_at,
    })
}

fn sort_reservations_for_listing(reservations: &mut [StoredReservation]) {
    reservations.sort_by(|left, right| {
        right.updated_at.cmp(&left.updated_at).then_with(|| {
            left.reservation
                .id
                .as_slice()
                .cmp(right.reservation.id.as_slice())
        })
    });
}

fn reservation_summary_from_stored(reservation: StoredReservation) -> ReservationSummaryResponse {
    ReservationSummaryResponse {
        reservation_id: encode_b256(reservation.reservation.id),
        status: reservation.reservation.status.into(),
        created_at: reservation.reservation.created_at,
        expires_at: reservation.reservation.expires_at,
        updated_at: reservation.updated_at,
        legs: reservation
            .reservation
            .legs
            .into_iter()
            .map(|leg| ReservationLegResponse {
                order_hash: encode_b256(leg.order_hash),
                role: ApiReservationLegRole::from(leg.role),
                claim_amount: encode_u256(leg.claim_amount),
            })
            .collect(),
    }
}

pub(crate) fn project_events(first_sequence: u64, events: &[EngineEvent]) -> Vec<ApiEvent> {
    events
        .iter()
        .enumerate()
        .map(|(offset, event)| ApiEvent::from_engine(first_sequence + offset as u64, event))
        .collect()
}
