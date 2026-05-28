#![forbid(unsafe_code)]

use std::collections::HashMap;

use asceswap_engine::{
    AsceSwapEngine, CancelOrder, CancelOrderResult, EngineError, EngineSnapshot, OrderRecord,
    OrderSnapshot, ReservationUpdateResult, SubmitOrder, SubmitOrderOutcome, SubmitOrderResult,
};
use asceswap_matcher::MatchConfig;
use asceswap_orderbook::DepthLevel;
use asceswap_state::{Reservation, ReservationId};
use asceswap_types::{ClaimSide, MarketId, OrderHash, Side};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarketActorError {
    InboxCapacityZero,
    InboxClosed,
    ResponseDropped,
    DuplicateMarket(MarketId),
    UnknownMarket(MarketId),
    MissingOrderRoute(OrderHash),
    MissingReservationRoute(ReservationId),
    EmptyReservation(ReservationId),
    ReservationOrderMissing {
        reservation_id: ReservationId,
        order_hash: OrderHash,
    },
    ReservationSpansMarkets {
        reservation_id: ReservationId,
        first: MarketId,
        second: MarketId,
    },
    WrongMarket {
        expected: MarketId,
        actual: MarketId,
    },
    Engine(EngineError),
}

impl From<EngineError> for MarketActorError {
    fn from(error: EngineError) -> Self {
        Self::Engine(error)
    }
}

#[derive(Clone, Debug, Default)]
pub struct MarketActorRouter {
    markets: HashMap<MarketId, MarketActorHandle>,
    order_routes: HashMap<OrderHash, MarketId>,
    reservation_routes: HashMap<ReservationId, MarketId>,
}

impl MarketActorRouter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn spawn_market(
        &mut self,
        market_id: MarketId,
        engine: AsceSwapEngine,
        inbox_capacity: usize,
    ) -> Result<(), MarketActorError> {
        if self.markets.contains_key(&market_id) {
            return Err(MarketActorError::DuplicateMarket(market_id));
        }

        let snapshot = engine.snapshot();
        let handle = spawn_market_actor(market_id, engine, inbox_capacity)?;
        self.index_snapshot(market_id, &snapshot);
        self.markets.insert(market_id, handle);

        Ok(())
    }

    pub fn spawn_from_snapshot(
        &mut self,
        snapshot: EngineSnapshot,
        match_config: MatchConfig,
        inbox_capacity: usize,
    ) -> Result<(), MarketActorError> {
        for (market_id, snapshot) in split_snapshot_by_market(snapshot)? {
            let engine = AsceSwapEngine::from_snapshot(match_config, snapshot)?;
            self.spawn_market(market_id, engine, inbox_capacity)?;
        }

        Ok(())
    }

    pub async fn submit_order(
        &mut self,
        command: SubmitOrder,
    ) -> Result<SubmitOrderResult, MarketActorError> {
        let market_id = command.order.market_id;
        let result = self.actor(market_id)?.submit_order(command).await?;
        self.index_submit_result(market_id, &result);

        Ok(result)
    }

    pub async fn cancel_order(
        &self,
        command: CancelOrder,
    ) -> Result<CancelOrderResult, MarketActorError> {
        let market_id = self
            .order_routes
            .get(&command.order_hash)
            .copied()
            .ok_or(MarketActorError::MissingOrderRoute(command.order_hash))?;
        self.actor(market_id)?.cancel_order(command).await
    }

    pub async fn mark_reservation_submitted(
        &self,
        reservation_id: ReservationId,
        now: u64,
    ) -> Result<ReservationUpdateResult, MarketActorError> {
        self.reservation_actor(reservation_id)?
            .mark_reservation_submitted(reservation_id, now)
            .await
    }

    pub async fn release_reservation(
        &self,
        reservation_id: ReservationId,
    ) -> Result<ReservationUpdateResult, MarketActorError> {
        self.reservation_actor(reservation_id)?
            .release_reservation(reservation_id)
            .await
    }

    pub async fn expire_reservation(
        &self,
        reservation_id: ReservationId,
        now: u64,
    ) -> Result<ReservationUpdateResult, MarketActorError> {
        self.reservation_actor(reservation_id)?
            .expire_reservation(reservation_id, now)
            .await
    }

    pub async fn commit_reservation(
        &self,
        reservation_id: ReservationId,
    ) -> Result<ReservationUpdateResult, MarketActorError> {
        self.reservation_actor(reservation_id)?
            .commit_reservation(reservation_id)
            .await
    }

    pub async fn order_record(
        &self,
        order_hash: OrderHash,
    ) -> Result<Option<OrderRecord>, MarketActorError> {
        let market_id = self
            .order_routes
            .get(&order_hash)
            .copied()
            .ok_or(MarketActorError::MissingOrderRoute(order_hash))?;
        self.actor(market_id)?.order_record(order_hash).await
    }

    pub async fn reservation(
        &self,
        reservation_id: ReservationId,
    ) -> Result<Option<Reservation>, MarketActorError> {
        self.reservation_actor(reservation_id)?
            .reservation(reservation_id)
            .await
    }

    pub async fn depth(
        &self,
        market_id: MarketId,
        claim: ClaimSide,
        side: Side,
    ) -> Result<Vec<DepthLevel>, MarketActorError> {
        self.actor(market_id)?.depth(claim, side).await
    }

    pub async fn snapshot(&self, market_id: MarketId) -> Result<EngineSnapshot, MarketActorError> {
        self.actor(market_id)?.snapshot().await
    }

    pub async fn snapshot_all(&self) -> Result<EngineSnapshot, MarketActorError> {
        let mut market_ids = self.markets.keys().copied().collect::<Vec<_>>();
        market_ids.sort_by(|left, right| left.as_slice().cmp(right.as_slice()));

        let mut orders = Vec::new();
        let mut reservations = Vec::new();
        let mut next_reservation_sequence = 0_u64;

        for market_id in market_ids {
            let mut snapshot = self.snapshot(market_id).await?;
            orders.append(&mut snapshot.orders);
            reservations.append(&mut snapshot.reservations);
            next_reservation_sequence =
                next_reservation_sequence.max(snapshot.next_reservation_sequence);
        }

        orders.sort_by(|left, right| left.hash.as_slice().cmp(right.hash.as_slice()));
        reservations.sort_by(|left, right| left.id.as_slice().cmp(right.id.as_slice()));

        Ok(EngineSnapshot {
            orders,
            reservations,
            next_reservation_sequence,
        })
    }

    pub fn market_count(&self) -> usize {
        self.markets.len()
    }

    pub fn contains_market(&self, market_id: MarketId) -> bool {
        self.markets.contains_key(&market_id)
    }

    fn actor(&self, market_id: MarketId) -> Result<&MarketActorHandle, MarketActorError> {
        self.markets
            .get(&market_id)
            .ok_or(MarketActorError::UnknownMarket(market_id))
    }

    fn reservation_actor(
        &self,
        reservation_id: ReservationId,
    ) -> Result<&MarketActorHandle, MarketActorError> {
        let market_id = self
            .reservation_routes
            .get(&reservation_id)
            .copied()
            .ok_or(MarketActorError::MissingReservationRoute(reservation_id))?;
        self.actor(market_id)
    }

    fn index_snapshot(&mut self, market_id: MarketId, snapshot: &EngineSnapshot) {
        for order in &snapshot.orders {
            self.order_routes.insert(order.hash, market_id);
        }
        for reservation in &snapshot.reservations {
            self.reservation_routes.insert(reservation.id, market_id);
        }
    }

    fn index_submit_result(&mut self, market_id: MarketId, result: &SubmitOrderResult) {
        self.order_routes.insert(result.order_hash, market_id);
        if let SubmitOrderOutcome::Matched { reservation_id, .. } = &result.outcome {
            self.reservation_routes.insert(*reservation_id, market_id);
        }
    }
}

#[derive(Clone, Debug)]
pub struct MarketActorHandle {
    market_id: MarketId,
    sender: mpsc::Sender<MarketActorMessage>,
}

impl MarketActorHandle {
    pub fn market_id(&self) -> MarketId {
        self.market_id
    }

    pub async fn submit_order(
        &self,
        command: SubmitOrder,
    ) -> Result<SubmitOrderResult, MarketActorError> {
        self.ensure_order_market(command.order.market_id)?;
        self.request(|respond_to| MarketActorMessage::SubmitOrder {
            command: Box::new(command),
            respond_to,
        })
        .await
    }

    pub async fn cancel_order(
        &self,
        command: CancelOrder,
    ) -> Result<CancelOrderResult, MarketActorError> {
        self.request(|respond_to| MarketActorMessage::CancelOrder {
            command,
            respond_to,
        })
        .await
    }

    pub async fn mark_reservation_submitted(
        &self,
        reservation_id: ReservationId,
        now: u64,
    ) -> Result<ReservationUpdateResult, MarketActorError> {
        self.request(|respond_to| MarketActorMessage::MarkReservationSubmitted {
            reservation_id,
            now,
            respond_to,
        })
        .await
    }

    pub async fn release_reservation(
        &self,
        reservation_id: ReservationId,
    ) -> Result<ReservationUpdateResult, MarketActorError> {
        self.request(|respond_to| MarketActorMessage::ReleaseReservation {
            reservation_id,
            respond_to,
        })
        .await
    }

    pub async fn expire_reservation(
        &self,
        reservation_id: ReservationId,
        now: u64,
    ) -> Result<ReservationUpdateResult, MarketActorError> {
        self.request(|respond_to| MarketActorMessage::ExpireReservation {
            reservation_id,
            now,
            respond_to,
        })
        .await
    }

    pub async fn commit_reservation(
        &self,
        reservation_id: ReservationId,
    ) -> Result<ReservationUpdateResult, MarketActorError> {
        self.request(|respond_to| MarketActorMessage::CommitReservation {
            reservation_id,
            respond_to,
        })
        .await
    }

    pub async fn order_record(
        &self,
        order_hash: OrderHash,
    ) -> Result<Option<OrderRecord>, MarketActorError> {
        self.request(|respond_to| MarketActorMessage::OrderRecord {
            order_hash,
            respond_to,
        })
        .await
    }

    pub async fn reservation(
        &self,
        reservation_id: ReservationId,
    ) -> Result<Option<Reservation>, MarketActorError> {
        self.request(|respond_to| MarketActorMessage::Reservation {
            reservation_id,
            respond_to,
        })
        .await
    }

    pub async fn depth(
        &self,
        claim: ClaimSide,
        side: Side,
    ) -> Result<Vec<DepthLevel>, MarketActorError> {
        self.request(|respond_to| MarketActorMessage::Depth {
            claim,
            side,
            respond_to,
        })
        .await
    }

    pub async fn snapshot(&self) -> Result<EngineSnapshot, MarketActorError> {
        self.request(MarketActorMessage::Snapshot).await
    }

    fn ensure_order_market(&self, actual: MarketId) -> Result<(), MarketActorError> {
        if actual != self.market_id {
            return Err(MarketActorError::WrongMarket {
                expected: self.market_id,
                actual,
            });
        }

        Ok(())
    }

    async fn request<T>(
        &self,
        build: impl FnOnce(oneshot::Sender<Result<T, MarketActorError>>) -> MarketActorMessage,
    ) -> Result<T, MarketActorError> {
        let (respond_to, response) = oneshot::channel();
        self.sender
            .send(build(respond_to))
            .await
            .map_err(|_| MarketActorError::InboxClosed)?;
        response
            .await
            .map_err(|_| MarketActorError::ResponseDropped)?
    }
}

pub fn spawn_market_actor(
    market_id: MarketId,
    engine: AsceSwapEngine,
    inbox_capacity: usize,
) -> Result<MarketActorHandle, MarketActorError> {
    if inbox_capacity == 0 {
        return Err(MarketActorError::InboxCapacityZero);
    }
    validate_engine_market(market_id, &engine)?;

    let (sender, receiver) = mpsc::channel(inbox_capacity);
    let actor = MarketActor { market_id, engine };
    tokio::spawn(actor.run(receiver));

    Ok(MarketActorHandle { market_id, sender })
}

struct MarketActor {
    market_id: MarketId,
    engine: AsceSwapEngine,
}

impl MarketActor {
    async fn run(mut self, mut receiver: mpsc::Receiver<MarketActorMessage>) {
        while let Some(message) = receiver.recv().await {
            self.handle(message);
        }
    }

    fn handle(&mut self, message: MarketActorMessage) {
        match message {
            MarketActorMessage::SubmitOrder {
                command,
                respond_to,
            } => {
                let result = self
                    .ensure_order_market(command.order.market_id)
                    .and_then(|()| self.engine.submit_order(*command).map_err(Into::into));
                send_response(respond_to, result);
            }
            MarketActorMessage::CancelOrder {
                command,
                respond_to,
            } => send_response(
                respond_to,
                self.engine.cancel_order(command).map_err(Into::into),
            ),
            MarketActorMessage::MarkReservationSubmitted {
                reservation_id,
                now,
                respond_to,
            } => send_response(
                respond_to,
                self.engine
                    .mark_reservation_submitted(reservation_id, now)
                    .map_err(Into::into),
            ),
            MarketActorMessage::ReleaseReservation {
                reservation_id,
                respond_to,
            } => send_response(
                respond_to,
                self.engine
                    .release_reservation(reservation_id)
                    .map_err(Into::into),
            ),
            MarketActorMessage::ExpireReservation {
                reservation_id,
                now,
                respond_to,
            } => send_response(
                respond_to,
                self.engine
                    .expire_reservation(reservation_id, now)
                    .map_err(Into::into),
            ),
            MarketActorMessage::CommitReservation {
                reservation_id,
                respond_to,
            } => send_response(
                respond_to,
                self.engine
                    .commit_reservation(reservation_id)
                    .map_err(Into::into),
            ),
            MarketActorMessage::OrderRecord {
                order_hash,
                respond_to,
            } => send_response(
                respond_to,
                Ok(self.engine.order_record(order_hash).cloned()),
            ),
            MarketActorMessage::Reservation {
                reservation_id,
                respond_to,
            } => send_response(
                respond_to,
                Ok(self.engine.reservation(reservation_id).cloned()),
            ),
            MarketActorMessage::Depth {
                claim,
                side,
                respond_to,
            } => {
                let result = self
                    .engine
                    .market_book(self.market_id)
                    .map(|book| book.depth(claim, side).map_err(EngineError::from))
                    .transpose()
                    .map(|levels| levels.unwrap_or_default())
                    .map_err(Into::into);
                send_response(respond_to, result);
            }
            MarketActorMessage::Snapshot(respond_to) => {
                send_response(respond_to, Ok(self.engine.snapshot()));
            }
        }
    }

    fn ensure_order_market(&self, actual: MarketId) -> Result<(), MarketActorError> {
        if actual != self.market_id {
            return Err(MarketActorError::WrongMarket {
                expected: self.market_id,
                actual,
            });
        }

        Ok(())
    }
}

enum MarketActorMessage {
    SubmitOrder {
        command: Box<SubmitOrder>,
        respond_to: oneshot::Sender<Result<SubmitOrderResult, MarketActorError>>,
    },
    CancelOrder {
        command: CancelOrder,
        respond_to: oneshot::Sender<Result<CancelOrderResult, MarketActorError>>,
    },
    MarkReservationSubmitted {
        reservation_id: ReservationId,
        now: u64,
        respond_to: oneshot::Sender<Result<ReservationUpdateResult, MarketActorError>>,
    },
    ReleaseReservation {
        reservation_id: ReservationId,
        respond_to: oneshot::Sender<Result<ReservationUpdateResult, MarketActorError>>,
    },
    ExpireReservation {
        reservation_id: ReservationId,
        now: u64,
        respond_to: oneshot::Sender<Result<ReservationUpdateResult, MarketActorError>>,
    },
    CommitReservation {
        reservation_id: ReservationId,
        respond_to: oneshot::Sender<Result<ReservationUpdateResult, MarketActorError>>,
    },
    OrderRecord {
        order_hash: OrderHash,
        respond_to: oneshot::Sender<Result<Option<OrderRecord>, MarketActorError>>,
    },
    Reservation {
        reservation_id: ReservationId,
        respond_to: oneshot::Sender<Result<Option<Reservation>, MarketActorError>>,
    },
    Depth {
        claim: ClaimSide,
        side: Side,
        respond_to: oneshot::Sender<Result<Vec<DepthLevel>, MarketActorError>>,
    },
    Snapshot(oneshot::Sender<Result<EngineSnapshot, MarketActorError>>),
}

fn validate_engine_market(
    market_id: MarketId,
    engine: &AsceSwapEngine,
) -> Result<(), MarketActorError> {
    for order in engine.snapshot().orders {
        if order.order.market_id != market_id {
            return Err(MarketActorError::WrongMarket {
                expected: market_id,
                actual: order.order.market_id,
            });
        }
    }

    Ok(())
}

fn split_snapshot_by_market(
    snapshot: EngineSnapshot,
) -> Result<Vec<(MarketId, EngineSnapshot)>, MarketActorError> {
    let mut market_orders = HashMap::<MarketId, Vec<OrderSnapshot>>::new();
    let mut order_markets = HashMap::<OrderHash, MarketId>::new();
    for order in snapshot.orders {
        let market_id = order.order.market_id;
        order_markets.insert(order.hash, market_id);
        market_orders.entry(market_id).or_default().push(order);
    }

    let mut market_reservations = HashMap::<MarketId, Vec<Reservation>>::new();
    for reservation in snapshot.reservations {
        let Some(first_leg) = reservation.legs.first() else {
            return Err(MarketActorError::EmptyReservation(reservation.id));
        };
        let first_market = *order_markets.get(&first_leg.order_hash).ok_or(
            MarketActorError::ReservationOrderMissing {
                reservation_id: reservation.id,
                order_hash: first_leg.order_hash,
            },
        )?;

        for leg in &reservation.legs[1..] {
            let leg_market = *order_markets.get(&leg.order_hash).ok_or(
                MarketActorError::ReservationOrderMissing {
                    reservation_id: reservation.id,
                    order_hash: leg.order_hash,
                },
            )?;
            if leg_market != first_market {
                return Err(MarketActorError::ReservationSpansMarkets {
                    reservation_id: reservation.id,
                    first: first_market,
                    second: leg_market,
                });
            }
        }

        market_reservations
            .entry(first_market)
            .or_default()
            .push(reservation);
    }

    let mut market_ids = market_orders.keys().copied().collect::<Vec<_>>();
    market_ids.sort_by(|left, right| left.as_slice().cmp(right.as_slice()));

    Ok(market_ids
        .into_iter()
        .map(|market_id| {
            let orders = market_orders.remove(&market_id).unwrap_or_default();
            let reservations = market_reservations.remove(&market_id).unwrap_or_default();
            (
                market_id,
                EngineSnapshot {
                    orders,
                    reservations,
                    next_reservation_sequence: snapshot.next_reservation_sequence,
                },
            )
        })
        .collect())
}

fn send_response<T>(
    respond_to: oneshot::Sender<Result<T, MarketActorError>>,
    result: Result<T, MarketActorError>,
) {
    let _ = respond_to.send(result);
}

#[cfg(test)]
mod tests;
