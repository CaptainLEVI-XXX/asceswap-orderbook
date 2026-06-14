use std::collections::{HashMap, HashSet};

use asceswap_matcher::{plan_match_with_filter, MatchConfig, MatchPlan};
use asceswap_math::{prepare_fill, remaining_claim_amount};
use asceswap_orderbook::MarketOrderBook;
use asceswap_state::{
    derive_reservation_id, OrderAvailability, OrderLifecycle, OrderState, Reservation,
    ReservationBook, ReservationId, ReservationLeg, ReservationLegRole, ReservationStatus,
    StateError,
};
use asceswap_types::{MarketId, Order, OrderHash, U256};
use asceswap_validation::{order_hash, validate_order};

use crate::{
    CancelOrder, CancelOrderResult, EngineError, EngineEvent, OrderRecord, ReservationUpdateResult,
    SettlementPayload, SubmitOrder, SubmitOrderOutcome, SubmitOrderResult,
};
use crate::{EngineSnapshot, OrderSnapshot};

#[derive(Clone, Debug)]
pub struct AsceSwapEngine {
    books: HashMap<MarketId, MarketOrderBook>,
    records: HashMap<OrderHash, OrderRecord>,
    reservations: ReservationBook,
    match_config: MatchConfig,
    next_reservation_sequence: u64,
}

impl Default for AsceSwapEngine {
    fn default() -> Self {
        Self::new(MatchConfig::default())
    }
}

impl AsceSwapEngine {
    pub fn new(match_config: MatchConfig) -> Self {
        Self {
            books: HashMap::new(),
            records: HashMap::new(),
            reservations: ReservationBook::new(),
            match_config,
            next_reservation_sequence: 0,
        }
    }

    pub fn order_record(&self, order_hash: OrderHash) -> Option<&OrderRecord> {
        self.records.get(&order_hash)
    }

    pub fn market_book(&self, market_id: MarketId) -> Option<&MarketOrderBook> {
        self.books.get(&market_id)
    }

    pub fn reservation(&self, reservation_id: ReservationId) -> Option<&Reservation> {
        self.reservations.get(reservation_id)
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        let mut orders = self
            .records
            .values()
            .map(|record| OrderSnapshot {
                hash: record.hash,
                order: record.order.clone(),
                signature: record.signature.clone(),
                state: record.state(),
                filled_claim_amount: record.filled_claim_amount,
                resting: record.resting,
                accepted_sequence: self.accepted_sequence(record),
            })
            .collect::<Vec<_>>();
        orders.sort_by(|left, right| left.hash.as_slice().cmp(right.hash.as_slice()));

        let mut reservations = self
            .reservations
            .reservations()
            .cloned()
            .collect::<Vec<_>>();
        reservations.sort_by(|left, right| left.id.as_slice().cmp(right.id.as_slice()));

        EngineSnapshot {
            orders,
            reservations,
            next_reservation_sequence: self.next_reservation_sequence,
        }
    }

    pub fn from_snapshot(
        match_config: MatchConfig,
        snapshot: EngineSnapshot,
    ) -> Result<Self, EngineError> {
        let mut books = HashMap::new();
        let mut records = HashMap::new();

        let mut resting_orders = Vec::new();

        for (fallback_sequence, order) in snapshot.orders.into_iter().enumerate() {
            let actual_hash = order_hash(&order.order);
            if actual_hash != order.hash {
                return Err(EngineError::SnapshotOrderHashMismatch {
                    expected: order.hash,
                    actual: actual_hash,
                });
            }

            if records.contains_key(&order.hash) {
                return Err(EngineError::DuplicateOrder(order.hash));
            }

            if order.resting {
                if !matches!(
                    order.state,
                    OrderState::Open
                        | OrderState::PartiallyFilled
                        | OrderState::Reserved
                        | OrderState::Submitted
                ) {
                    return Err(EngineError::InvalidOrderState {
                        order_hash: order.hash,
                        state: order.state,
                    });
                }

                let accepted_sequence = order.accepted_sequence.unwrap_or(
                    u64::try_from(fallback_sequence).map_err(|_| EngineError::TimeOverflow)?,
                );
                resting_orders.push((accepted_sequence, order.clone()));
            }

            records.insert(
                order.hash,
                OrderRecord::new(
                    order.hash,
                    order.order,
                    order.state,
                    order.filled_claim_amount,
                    order.resting,
                )
                .with_signature(order.signature),
            );
        }

        resting_orders.sort_by(|(left_sequence, left), (right_sequence, right)| {
            left_sequence
                .cmp(right_sequence)
                .then_with(|| left.hash.as_slice().cmp(right.hash.as_slice()))
        });

        for (accepted_sequence, order) in resting_orders {
            let book = books
                .entry(order.order.market_id)
                .or_insert_with(|| MarketOrderBook::new(order.order.market_id));
            book.restore(
                order.hash,
                order.order,
                order.filled_claim_amount,
                accepted_sequence,
            )?;
        }

        validate_snapshot_reservations(&records, &snapshot.reservations)?;

        Ok(Self {
            books,
            records,
            reservations: ReservationBook::from_reservations(snapshot.reservations)?,
            match_config,
            next_reservation_sequence: snapshot.next_reservation_sequence,
        })
    }

    pub fn submit_order(&mut self, command: SubmitOrder) -> Result<SubmitOrderResult, EngineError> {
        let order_hash = order_hash(&command.order);
        if self.records.contains_key(&order_hash) {
            return Err(EngineError::DuplicateOrder(order_hash));
        }

        let market_id = command.order.market_id;
        let mut events = vec![EngineEvent::OrderReceived {
            order_hash,
            market_id,
        }];

        let mut lifecycle = OrderLifecycle::new(OrderState::Received);
        lifecycle.transition_to(OrderState::Validating)?;

        let validated = match validate_order(&command.order, &command.validation) {
            Ok(validated) => validated,
            Err(reason) => {
                lifecycle.transition_to(OrderState::Rejected)?;
                self.records.insert(
                    order_hash,
                    OrderRecord::new(
                        order_hash,
                        command.order,
                        OrderState::Rejected,
                        command.validation.filled_claim_amount,
                        false,
                    )
                    .with_signature(command.signature),
                );
                events.push(EngineEvent::OrderRejected { order_hash, reason });
                return Ok(SubmitOrderResult {
                    order_hash,
                    outcome: SubmitOrderOutcome::Rejected { reason },
                    events,
                });
            }
        };

        events.push(EngineEvent::OrderValidated {
            order_hash,
            remaining_claim_amount: validated.remaining_claim_amount,
        });

        let plan = self.plan_for_submit(
            &command.order,
            validated.filled_claim_amount,
            command.validation.now,
        )?;

        if command.post_only && plan.is_some() {
            lifecycle.transition_to(OrderState::Inactive)?;
            self.records.insert(
                order_hash,
                OrderRecord::new(
                    order_hash,
                    command.order,
                    OrderState::Inactive,
                    validated.filled_claim_amount,
                    false,
                )
                .with_signature(command.signature),
            );
            events.push(EngineEvent::OrderInactive { order_hash });
            return Ok(SubmitOrderResult {
                order_hash,
                outcome: SubmitOrderOutcome::PostOnlyWouldCross,
                events,
            });
        }

        if let Some(plan) = plan {
            let settlement = self.settlement_payload_for_plan(
                &command.order,
                command.signature.as_deref(),
                &plan,
            );
            self.ensure_plan_makers_reservable(&plan)?;
            let reservation_id = self.create_reservation(
                order_hash,
                &command.order,
                validated.filled_claim_amount,
                &plan,
                command.validation.now,
                command.reservation_ttl_secs,
            )?;

            lifecycle.transition_to(OrderState::Reserved)?;
            self.records.insert(
                order_hash,
                OrderRecord::new(
                    order_hash,
                    command.order,
                    OrderState::Reserved,
                    validated.filled_claim_amount,
                    false,
                )
                .with_signature(command.signature),
            );

            events.push(EngineEvent::OrderReserved {
                order_hash,
                reservation_id,
            });
            for maker_fill in &plan.maker_fills {
                let maker_record = self
                    .records
                    .get_mut(&maker_fill.order_hash)
                    .ok_or(EngineError::MissingOrder(maker_fill.order_hash))?;
                maker_record.transition_to(OrderState::Reserved)?;
                events.push(EngineEvent::OrderReserved {
                    order_hash: maker_fill.order_hash,
                    reservation_id,
                });
            }
            events.push(EngineEvent::ReservationCreated {
                reservation_id,
                match_kind: plan.match_kind,
                maker_count: plan.maker_fills.len(),
            });

            return Ok(SubmitOrderResult {
                order_hash,
                outcome: SubmitOrderOutcome::Matched {
                    reservation_id,
                    plan,
                    settlement,
                },
                events,
            });
        }

        if !command.rest_on_no_match {
            lifecycle.transition_to(OrderState::Inactive)?;
            self.records.insert(
                order_hash,
                OrderRecord::new(
                    order_hash,
                    command.order,
                    OrderState::Inactive,
                    validated.filled_claim_amount,
                    false,
                )
                .with_signature(command.signature),
            );
            events.push(EngineEvent::OrderInactive { order_hash });
            return Ok(SubmitOrderResult {
                order_hash,
                outcome: SubmitOrderOutcome::Inactive,
                events,
            });
        }

        lifecycle.transition_to(OrderState::Open)?;
        let price = self
            .books
            .entry(market_id)
            .or_insert_with(|| MarketOrderBook::new(market_id))
            .insert(order_hash, command.order.clone())?;
        self.records.insert(
            order_hash,
            OrderRecord::new(
                order_hash,
                command.order,
                OrderState::Open,
                validated.filled_claim_amount,
                true,
            )
            .with_signature(command.signature),
        );
        events.push(EngineEvent::OrderOpened { order_hash });

        Ok(SubmitOrderResult {
            order_hash,
            outcome: SubmitOrderOutcome::Rested { price },
            events,
        })
    }

    fn accepted_sequence(&self, record: &OrderRecord) -> Option<u64> {
        if !record.resting {
            return None;
        }

        self.books
            .get(&record.order.market_id)?
            .get(record.hash)
            .map(|order| order.accepted_sequence)
    }

    pub fn cancel_order(&mut self, command: CancelOrder) -> Result<CancelOrderResult, EngineError> {
        let (market_id, was_resting) = {
            let record = self
                .records
                .get(&command.order_hash)
                .ok_or(EngineError::MissingOrder(command.order_hash))?;
            (record.order.market_id, record.resting)
        };

        if was_resting {
            let book = self
                .books
                .get_mut(&market_id)
                .ok_or(EngineError::MissingMarket(market_id))?;
            book.remove(command.order_hash)?;
        }

        let record = self
            .records
            .get_mut(&command.order_hash)
            .ok_or(EngineError::MissingOrder(command.order_hash))?;
        record.resting = false;
        record.transition_to(OrderState::Cancelled)?;

        Ok(CancelOrderResult {
            order_hash: command.order_hash,
            events: vec![EngineEvent::OrderCancelled {
                order_hash: command.order_hash,
            }],
        })
    }

    pub fn mark_reservation_submitted(
        &mut self,
        reservation_id: ReservationId,
        now: u64,
    ) -> Result<ReservationUpdateResult, EngineError> {
        if self
            .reservations
            .get(reservation_id)
            .ok_or(EngineError::State(StateError::MissingReservation(
                reservation_id,
            )))?
            .is_expired(now)
        {
            self.expire_reservation(reservation_id, now)?;
            return Err(EngineError::ReservationExpired(reservation_id));
        }

        let reservation = self
            .reservations
            .mark_submitted_at(reservation_id, now)?
            .clone();
        let mut events = vec![EngineEvent::ReservationSubmitted { reservation_id }];
        for leg in &reservation.legs {
            let record = self.record_mut(leg.order_hash)?;
            record.transition_to(OrderState::Submitted)?;
            events.push(EngineEvent::OrderSubmitted {
                order_hash: leg.order_hash,
                reservation_id,
            });
        }

        Ok(ReservationUpdateResult {
            reservation_id,
            events,
        })
    }

    pub fn release_reservation(
        &mut self,
        reservation_id: ReservationId,
    ) -> Result<ReservationUpdateResult, EngineError> {
        let reservation = self
            .reservations
            .get(reservation_id)
            .ok_or(EngineError::State(StateError::MissingReservation(
                reservation_id,
            )))?
            .clone();
        self.reservations.release(reservation_id)?;

        let mut events = vec![EngineEvent::ReservationReleased { reservation_id }];
        self.restore_leg_states(&reservation.legs, &mut events)?;

        Ok(ReservationUpdateResult {
            reservation_id,
            events,
        })
    }

    pub fn expire_reservation(
        &mut self,
        reservation_id: ReservationId,
        now: u64,
    ) -> Result<ReservationUpdateResult, EngineError> {
        let reservation = self
            .reservations
            .get(reservation_id)
            .ok_or(EngineError::State(StateError::MissingReservation(
                reservation_id,
            )))?
            .clone();
        self.reservations.expire(reservation_id, now)?;

        let mut events = vec![EngineEvent::ReservationExpired { reservation_id }];
        self.restore_leg_states(&reservation.legs, &mut events)?;

        Ok(ReservationUpdateResult {
            reservation_id,
            events,
        })
    }

    pub fn expire_expired_reservations(
        &mut self,
        now: u64,
    ) -> Result<Vec<ReservationUpdateResult>, EngineError> {
        let expired_ids = self.reservations.expire_expired(now)?;
        let mut results = Vec::with_capacity(expired_ids.len());

        for reservation_id in expired_ids {
            let reservation = self
                .reservations
                .get(reservation_id)
                .ok_or(EngineError::State(StateError::MissingReservation(
                    reservation_id,
                )))?
                .clone();
            let mut events = vec![EngineEvent::ReservationExpired { reservation_id }];
            self.restore_leg_states(&reservation.legs, &mut events)?;
            results.push(ReservationUpdateResult {
                reservation_id,
                events,
            });
        }

        Ok(results)
    }

    pub fn commit_reservation(
        &mut self,
        reservation_id: ReservationId,
    ) -> Result<ReservationUpdateResult, EngineError> {
        let reservation = self
            .reservations
            .get(reservation_id)
            .ok_or(EngineError::State(StateError::MissingReservation(
                reservation_id,
            )))?
            .clone();
        if reservation.status != ReservationStatus::Submitted {
            return Err(EngineError::State(StateError::ReservationNotActive {
                reservation_id,
                status: reservation.status,
            }));
        }

        self.reservations.commit(reservation_id)?;

        let mut events = vec![EngineEvent::ReservationCommitted { reservation_id }];
        for leg in &reservation.legs {
            self.apply_committed_fill(leg, &mut events)?;
        }

        Ok(ReservationUpdateResult {
            reservation_id,
            events,
        })
    }

    pub fn settlement_payload(
        &self,
        reservation_id: ReservationId,
    ) -> Result<SettlementPayload, EngineError> {
        let reservation = self
            .reservations
            .get(reservation_id)
            .ok_or(EngineError::State(StateError::MissingReservation(
                reservation_id,
            )))?;

        let mut taker = None;
        let mut makers = Vec::new();
        for leg in &reservation.legs {
            match leg.role {
                ReservationLegRole::Taker => taker = Some(leg),
                ReservationLegRole::Maker => makers.push(leg),
            }
        }

        let taker = taker.ok_or(EngineError::InvalidReservationForSettlement(reservation_id))?;
        let taker_record = self.record(taker.order_hash)?;
        let taker_signature = taker_record
            .signature
            .clone()
            .ok_or(EngineError::MissingOrderSignature(taker.order_hash))?;

        let mut maker_orders = Vec::with_capacity(makers.len());
        let mut maker_signatures = Vec::with_capacity(makers.len());
        let mut maker_claim_fill_amounts = Vec::with_capacity(makers.len());
        for maker in makers {
            let record = self.record(maker.order_hash)?;
            maker_orders.push(record.order.clone());
            maker_signatures.push(
                record
                    .signature
                    .clone()
                    .ok_or(EngineError::MissingOrderSignature(maker.order_hash))?,
            );
            maker_claim_fill_amounts.push(maker.claim_amount);
        }

        Ok(SettlementPayload {
            taker_order: taker_record.order.clone(),
            taker_signature,
            maker_orders,
            maker_signatures,
            taker_claim_fill_amount: taker.claim_amount,
            maker_claim_fill_amounts,
        })
    }

    fn create_reservation(
        &mut self,
        taker_order_hash: OrderHash,
        taker_order: &Order,
        taker_filled_claim_amount: U256,
        plan: &MatchPlan,
        now: u64,
        ttl_secs: Option<u64>,
    ) -> Result<ReservationId, EngineError> {
        let availability = self.availability_for_plan(
            taker_order_hash,
            taker_order,
            taker_filled_claim_amount,
            plan,
        )?;
        let expires_at = ttl_secs
            .map(|ttl| now.checked_add(ttl).ok_or(EngineError::TimeOverflow))
            .transpose()?;
        let reservation_id =
            derive_reservation_id(taker_order_hash, plan, self.next_reservation_sequence);
        self.reservations.create_with_expiration(
            reservation_id,
            taker_order_hash,
            plan,
            &availability,
            now,
            expires_at,
        )?;
        self.next_reservation_sequence = self
            .next_reservation_sequence
            .checked_add(1)
            .ok_or(EngineError::TimeOverflow)?;

        Ok(reservation_id)
    }

    fn availability_for_plan(
        &self,
        taker_order_hash: OrderHash,
        taker_order: &Order,
        taker_filled_claim_amount: U256,
        plan: &MatchPlan,
    ) -> Result<HashMap<OrderHash, OrderAvailability>, EngineError> {
        let mut availability = HashMap::with_capacity(plan.maker_fills.len() + 1);
        availability.insert(
            taker_order_hash,
            OrderAvailability {
                filled_claim_amount: taker_filled_claim_amount,
                max_claim_amount: taker_order.max_claim_amount(),
            },
        );

        for maker_fill in &plan.maker_fills {
            let record = self
                .records
                .get(&maker_fill.order_hash)
                .ok_or(EngineError::MissingOrder(maker_fill.order_hash))?;
            availability.insert(
                maker_fill.order_hash,
                OrderAvailability {
                    filled_claim_amount: record.filled_claim_amount,
                    max_claim_amount: record.order.max_claim_amount(),
                },
            );
        }

        Ok(availability)
    }

    fn ensure_plan_makers_reservable(&self, plan: &MatchPlan) -> Result<(), EngineError> {
        for maker_fill in &plan.maker_fills {
            let record = self.record(maker_fill.order_hash)?;
            if !matches!(
                record.state(),
                OrderState::Open | OrderState::PartiallyFilled
            ) {
                return Err(EngineError::InvalidOrderState {
                    order_hash: maker_fill.order_hash,
                    state: record.state(),
                });
            }
        }

        Ok(())
    }

    fn plan_for_submit(
        &mut self,
        order: &Order,
        filled_claim_amount: U256,
        now: u64,
    ) -> Result<Option<MatchPlan>, EngineError> {
        let unavailable_maker_hashes = self.unavailable_maker_hashes();
        let now = U256::from(now);
        let book = self
            .books
            .entry(order.market_id)
            .or_insert_with(|| MarketOrderBook::new(order.market_id));
        Ok(plan_match_with_filter(
            book,
            order,
            filled_claim_amount,
            self.match_config,
            |maker| {
                !unavailable_maker_hashes.contains(&maker.hash)
                    && (maker.order.expiration == U256::ZERO || maker.order.expiration >= now)
            },
        )?)
    }

    fn settlement_payload_for_plan(
        &self,
        taker_order: &Order,
        taker_signature: Option<&[u8]>,
        plan: &MatchPlan,
    ) -> Option<SettlementPayload> {
        let mut maker_orders = Vec::with_capacity(plan.maker_fills.len());
        let mut maker_signatures = Vec::with_capacity(plan.maker_fills.len());
        let mut maker_claim_fill_amounts = Vec::with_capacity(plan.maker_fills.len());

        for maker_fill in &plan.maker_fills {
            let record = self.records.get(&maker_fill.order_hash)?;
            maker_orders.push(record.order.clone());
            maker_signatures.push(record.signature.clone()?);
            maker_claim_fill_amounts.push(maker_fill.claim_fill_amount);
        }

        Some(SettlementPayload {
            taker_order: taker_order.clone(),
            taker_signature: taker_signature?.to_vec(),
            maker_orders,
            maker_signatures,
            taker_claim_fill_amount: plan.taker_claim_fill_amount,
            maker_claim_fill_amounts,
        })
    }

    fn unavailable_maker_hashes(&self) -> HashSet<OrderHash> {
        self.records
            .iter()
            .filter_map(|(order_hash, record)| {
                matches!(record.state(), OrderState::Reserved | OrderState::Submitted)
                    .then_some(*order_hash)
            })
            .collect()
    }

    fn restore_leg_states(
        &mut self,
        legs: &[ReservationLeg],
        events: &mut Vec<EngineEvent>,
    ) -> Result<(), EngineError> {
        for leg in legs {
            let record = self.record_mut(leg.order_hash)?;
            let state = released_state(record);
            record.transition_to(state)?;
            events.push(EngineEvent::OrderStateChanged {
                order_hash: leg.order_hash,
                state,
            });
        }

        Ok(())
    }

    fn apply_committed_fill(
        &mut self,
        leg: &ReservationLeg,
        events: &mut Vec<EngineEvent>,
    ) -> Result<(), EngineError> {
        let (market_id, new_filled_claim_amount, fully_filled) = {
            let record = self.record_mut(leg.order_hash)?;
            let fill = prepare_fill(&record.order, record.filled_claim_amount, leg.claim_amount)?;
            let fully_filled = fill.new_filled_claim_amount == record.order.max_claim_amount();
            (
                record.order.market_id,
                fill.new_filled_claim_amount,
                fully_filled,
            )
        };

        let was_resting = self.record(leg.order_hash)?.resting;
        if was_resting {
            let book = self
                .books
                .get_mut(&market_id)
                .ok_or(EngineError::MissingMarket(market_id))?;
            book.apply_fill(leg.order_hash, leg.claim_amount)?;
        }

        let record = self.record_mut(leg.order_hash)?;
        record.filled_claim_amount = new_filled_claim_amount;
        if fully_filled {
            record.resting = false;
            record.transition_to(OrderState::Filled)?;
            events.push(EngineEvent::OrderFilled {
                order_hash: leg.order_hash,
            });
        } else {
            record.transition_to(OrderState::PartiallyFilled)?;
            let remaining = remaining_claim_amount(&record.order, record.filled_claim_amount)?;
            events.push(EngineEvent::OrderPartiallyFilled {
                order_hash: leg.order_hash,
                filled_claim_amount: record.filled_claim_amount,
                remaining_claim_amount: remaining,
            });
        }

        Ok(())
    }

    fn record(&self, order_hash: OrderHash) -> Result<&OrderRecord, EngineError> {
        self.records
            .get(&order_hash)
            .ok_or(EngineError::MissingOrder(order_hash))
    }

    fn record_mut(&mut self, order_hash: OrderHash) -> Result<&mut OrderRecord, EngineError> {
        self.records
            .get_mut(&order_hash)
            .ok_or(EngineError::MissingOrder(order_hash))
    }
}

fn released_state(record: &OrderRecord) -> OrderState {
    if record.resting {
        if record.filled_claim_amount == U256::ZERO {
            OrderState::Open
        } else {
            OrderState::PartiallyFilled
        }
    } else if record.filled_claim_amount == U256::ZERO {
        OrderState::Inactive
    } else {
        OrderState::PartiallyFilled
    }
}

fn validate_snapshot_reservations(
    records: &HashMap<OrderHash, OrderRecord>,
    reservations: &[Reservation],
) -> Result<(), EngineError> {
    let mut reserved_by_order = HashMap::new();

    for reservation in reservations {
        let is_active = matches!(
            reservation.status,
            ReservationStatus::Reserved | ReservationStatus::Submitted
        );

        for leg in &reservation.legs {
            let record = records
                .get(&leg.order_hash)
                .ok_or(EngineError::MissingOrder(leg.order_hash))?;

            if is_active {
                if !matches!(record.state(), OrderState::Reserved | OrderState::Submitted) {
                    return Err(EngineError::InvalidOrderState {
                        order_hash: leg.order_hash,
                        state: record.state(),
                    });
                }

                let current = reserved_by_order
                    .get(&leg.order_hash)
                    .copied()
                    .unwrap_or(U256::ZERO);
                let next = current
                    .checked_add(leg.claim_amount)
                    .ok_or(EngineError::ArithmeticOverflow)?;
                reserved_by_order.insert(leg.order_hash, next);
            }
        }
    }

    for (order_hash, reserved) in reserved_by_order {
        let record = records
            .get(&order_hash)
            .ok_or(EngineError::MissingOrder(order_hash))?;
        let available = remaining_claim_amount(&record.order, record.filled_claim_amount)?;
        if reserved > available {
            return Err(EngineError::ReservedAmountExceedsAvailable {
                order_hash,
                reserved,
                available,
            });
        }
    }

    Ok(())
}
