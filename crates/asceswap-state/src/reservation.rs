use std::collections::HashMap;

use alloy_primitives::keccak256;
use asceswap_matcher::MatchPlan;
use asceswap_types::{MatchKind, OrderHash, B256, U256};

use crate::StateError;

pub type ReservationId = B256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderAvailability {
    pub filled_claim_amount: U256,
    pub max_claim_amount: U256,
}

impl OrderAvailability {
    pub fn available_claim_amount(self) -> Result<U256, StateError> {
        if self.filled_claim_amount > self.max_claim_amount {
            return Ok(U256::ZERO);
        }

        Ok(self.max_claim_amount - self.filled_claim_amount)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReservationLegRole {
    Taker,
    Maker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReservationLeg {
    pub order_hash: OrderHash,
    pub role: ReservationLegRole,
    pub claim_amount: U256,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReservationStatus {
    Reserved,
    Submitted,
    Released,
    Expired,
    Committed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reservation {
    pub id: ReservationId,
    pub status: ReservationStatus,
    pub created_at: u64,
    pub expires_at: Option<u64>,
    pub legs: Vec<ReservationLeg>,
}

impl Reservation {
    pub fn is_expired(&self, now: u64) -> bool {
        matches!(self.expires_at, Some(expires_at) if now >= expires_at)
    }
}

#[derive(Clone, Debug, Default)]
pub struct ReservationBook {
    reservations: HashMap<ReservationId, Reservation>,
    reserved_by_order: HashMap<OrderHash, U256>,
}

impl ReservationBook {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_reservations(reservations: Vec<Reservation>) -> Result<Self, StateError> {
        let mut book = Self::new();
        for reservation in reservations {
            if let Some(expires_at) = reservation.expires_at {
                if expires_at <= reservation.created_at {
                    return Err(StateError::InvalidReservationExpiry {
                        created_at: reservation.created_at,
                        expires_at,
                    });
                }
            }

            if book.reservations.contains_key(&reservation.id) {
                return Err(StateError::DuplicateReservation(reservation.id));
            }

            if matches!(
                reservation.status,
                ReservationStatus::Reserved | ReservationStatus::Submitted
            ) {
                book.add_reserved(&reservation.legs)?;
            }

            book.reservations.insert(reservation.id, reservation);
        }

        Ok(book)
    }

    pub fn reservations(&self) -> impl Iterator<Item = &Reservation> {
        self.reservations.values()
    }

    pub fn reserved_claim_amount(&self, order_hash: OrderHash) -> U256 {
        self.reserved_by_order
            .get(&order_hash)
            .copied()
            .unwrap_or(U256::ZERO)
    }

    pub fn get(&self, reservation_id: ReservationId) -> Option<&Reservation> {
        self.reservations.get(&reservation_id)
    }

    pub fn create(
        &mut self,
        reservation_id: ReservationId,
        taker_order_hash: OrderHash,
        plan: &MatchPlan,
        availability: &HashMap<OrderHash, OrderAvailability>,
    ) -> Result<&Reservation, StateError> {
        self.create_with_expiration(
            reservation_id,
            taker_order_hash,
            plan,
            availability,
            0,
            None,
        )
    }

    pub fn create_with_expiration(
        &mut self,
        reservation_id: ReservationId,
        taker_order_hash: OrderHash,
        plan: &MatchPlan,
        availability: &HashMap<OrderHash, OrderAvailability>,
        created_at: u64,
        expires_at: Option<u64>,
    ) -> Result<&Reservation, StateError> {
        if let Some(expires_at) = expires_at {
            if expires_at <= created_at {
                return Err(StateError::InvalidReservationExpiry {
                    created_at,
                    expires_at,
                });
            }
        }

        if self.reservations.contains_key(&reservation_id) {
            return Err(StateError::DuplicateReservation(reservation_id));
        }

        let mut legs = Vec::with_capacity(plan.maker_fills.len() + 1);
        legs.push(ReservationLeg {
            order_hash: taker_order_hash,
            role: ReservationLegRole::Taker,
            claim_amount: plan.taker_claim_fill_amount,
        });
        for maker_fill in &plan.maker_fills {
            legs.push(ReservationLeg {
                order_hash: maker_fill.order_hash,
                role: ReservationLegRole::Maker,
                claim_amount: maker_fill.claim_fill_amount,
            });
        }

        self.ensure_available(&legs, availability)?;
        self.add_reserved(&legs)?;

        self.reservations.insert(
            reservation_id,
            Reservation {
                id: reservation_id,
                status: ReservationStatus::Reserved,
                created_at,
                expires_at,
                legs,
            },
        );

        Ok(self
            .reservations
            .get(&reservation_id)
            .expect("reservation inserted"))
    }

    pub fn mark_submitted(
        &mut self,
        reservation_id: ReservationId,
    ) -> Result<&Reservation, StateError> {
        self.mark_submitted_inner(reservation_id)
    }

    pub fn mark_submitted_at(
        &mut self,
        reservation_id: ReservationId,
        now: u64,
    ) -> Result<&Reservation, StateError> {
        if self.is_expired(reservation_id, now)? {
            self.expire(reservation_id, now)?;
            return Err(StateError::ReservationNotActive {
                reservation_id,
                status: ReservationStatus::Expired,
            });
        }

        self.mark_submitted_inner(reservation_id)
    }

    fn mark_submitted_inner(
        &mut self,
        reservation_id: ReservationId,
    ) -> Result<&Reservation, StateError> {
        let reservation = self
            .reservations
            .get_mut(&reservation_id)
            .ok_or(StateError::MissingReservation(reservation_id))?;
        if reservation.status != ReservationStatus::Reserved {
            return Err(StateError::ReservationNotActive {
                reservation_id,
                status: reservation.status,
            });
        }

        reservation.status = ReservationStatus::Submitted;
        Ok(reservation)
    }

    pub fn release(&mut self, reservation_id: ReservationId) -> Result<&Reservation, StateError> {
        let legs = {
            let reservation = self
                .reservations
                .get(&reservation_id)
                .ok_or(StateError::MissingReservation(reservation_id))?;
            if !matches!(
                reservation.status,
                ReservationStatus::Reserved | ReservationStatus::Submitted
            ) {
                return Err(StateError::ReservationNotActive {
                    reservation_id,
                    status: reservation.status,
                });
            }
            reservation.legs.clone()
        };

        self.subtract_reserved(&legs)?;
        let reservation = self
            .reservations
            .get_mut(&reservation_id)
            .expect("reservation checked above");
        reservation.status = ReservationStatus::Released;
        Ok(reservation)
    }

    pub fn commit(&mut self, reservation_id: ReservationId) -> Result<&Reservation, StateError> {
        let legs = {
            let reservation = self
                .reservations
                .get(&reservation_id)
                .ok_or(StateError::MissingReservation(reservation_id))?;
            if reservation.status != ReservationStatus::Submitted {
                return Err(StateError::ReservationNotActive {
                    reservation_id,
                    status: reservation.status,
                });
            }
            reservation.legs.clone()
        };

        self.subtract_reserved(&legs)?;
        let reservation = self
            .reservations
            .get_mut(&reservation_id)
            .expect("reservation checked above");
        reservation.status = ReservationStatus::Committed;
        Ok(reservation)
    }

    pub fn expire(
        &mut self,
        reservation_id: ReservationId,
        now: u64,
    ) -> Result<&Reservation, StateError> {
        let legs = {
            let reservation = self
                .reservations
                .get(&reservation_id)
                .ok_or(StateError::MissingReservation(reservation_id))?;
            if reservation.status != ReservationStatus::Reserved {
                return Err(StateError::ReservationNotActive {
                    reservation_id,
                    status: reservation.status,
                });
            }
            if !reservation.is_expired(now) {
                return Err(StateError::ReservationNotExpired {
                    reservation_id,
                    expires_at: reservation.expires_at,
                    now,
                });
            }
            reservation.legs.clone()
        };

        self.subtract_reserved(&legs)?;
        let reservation = self
            .reservations
            .get_mut(&reservation_id)
            .expect("reservation checked above");
        reservation.status = ReservationStatus::Expired;
        Ok(reservation)
    }

    pub fn expire_expired(&mut self, now: u64) -> Result<Vec<ReservationId>, StateError> {
        let expired_ids = self
            .reservations
            .iter()
            .filter_map(|(reservation_id, reservation)| {
                if reservation.status == ReservationStatus::Reserved && reservation.is_expired(now)
                {
                    Some(*reservation_id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        for reservation_id in &expired_ids {
            self.expire(*reservation_id, now)?;
        }

        Ok(expired_ids)
    }

    fn is_expired(&self, reservation_id: ReservationId, now: u64) -> Result<bool, StateError> {
        let reservation = self
            .reservations
            .get(&reservation_id)
            .ok_or(StateError::MissingReservation(reservation_id))?;
        Ok(reservation.is_expired(now))
    }

    fn ensure_available(
        &self,
        legs: &[ReservationLeg],
        availability: &HashMap<OrderHash, OrderAvailability>,
    ) -> Result<(), StateError> {
        let requested_by_order = aggregate_legs(legs)?;
        for (order_hash, requested) in requested_by_order {
            let availability = availability
                .get(&order_hash)
                .ok_or(StateError::MissingAvailability(order_hash))?;
            let live_available = availability.available_claim_amount()?;
            let already_reserved = self.reserved_claim_amount(order_hash);
            let available_after_reserved = live_available.checked_sub(already_reserved).ok_or(
                StateError::InsufficientAvailable {
                    order_hash,
                    requested,
                    available: U256::ZERO,
                },
            )?;

            if requested > available_after_reserved {
                return Err(StateError::InsufficientAvailable {
                    order_hash,
                    requested,
                    available: available_after_reserved,
                });
            }
        }

        Ok(())
    }

    fn add_reserved(&mut self, legs: &[ReservationLeg]) -> Result<(), StateError> {
        for (order_hash, claim_amount) in aggregate_legs(legs)? {
            let current = self.reserved_claim_amount(order_hash);
            let next = current
                .checked_add(claim_amount)
                .ok_or(StateError::ArithmeticOverflow)?;
            self.reserved_by_order.insert(order_hash, next);
        }

        Ok(())
    }

    fn subtract_reserved(&mut self, legs: &[ReservationLeg]) -> Result<(), StateError> {
        for (order_hash, claim_amount) in aggregate_legs(legs)? {
            let current = self.reserved_claim_amount(order_hash);
            let next = current
                .checked_sub(claim_amount)
                .ok_or(StateError::ArithmeticOverflow)?;
            if next == U256::ZERO {
                self.reserved_by_order.remove(&order_hash);
            } else {
                self.reserved_by_order.insert(order_hash, next);
            }
        }

        Ok(())
    }
}

pub fn derive_reservation_id(
    taker_order_hash: OrderHash,
    plan: &MatchPlan,
    sequence: u64,
) -> ReservationId {
    let mut encoded = Vec::with_capacity(32 * (8 + plan.maker_fills.len() * 4));
    encoded.extend_from_slice(b"ASCESWAP_RESERVATION_V1");
    push_b256(&mut encoded, taker_order_hash);
    push_u256(&mut encoded, U256::from(sequence));
    push_u256(&mut encoded, U256::from(match_kind_word(plan.match_kind)));
    push_u256(&mut encoded, plan.taker_claim_fill_amount);
    push_u256(&mut encoded, plan.taker_collateral_amount);
    push_u256(&mut encoded, plan.taker_actual_collateral_amount);
    push_u256(&mut encoded, plan.total_maker_claim_fill_amount);
    push_u256(&mut encoded, plan.total_maker_collateral_amount);
    push_u256(&mut encoded, U256::from(plan.maker_fills.len()));

    for maker_fill in &plan.maker_fills {
        push_b256(&mut encoded, maker_fill.order_hash);
        push_u256(&mut encoded, maker_fill.claim_fill_amount);
        push_u256(&mut encoded, maker_fill.collateral_amount);
        push_u256(&mut encoded, maker_fill.new_filled_claim_amount);
    }

    keccak256(encoded)
}

fn aggregate_legs(legs: &[ReservationLeg]) -> Result<HashMap<OrderHash, U256>, StateError> {
    let mut requested_by_order = HashMap::new();
    for leg in legs {
        let current = requested_by_order
            .get(&leg.order_hash)
            .copied()
            .unwrap_or(U256::ZERO);
        let next = current
            .checked_add(leg.claim_amount)
            .ok_or(StateError::ArithmeticOverflow)?;
        requested_by_order.insert(leg.order_hash, next);
    }

    Ok(requested_by_order)
}

fn push_b256(encoded: &mut Vec<u8>, value: B256) {
    encoded.extend_from_slice(value.as_slice());
}

fn push_u256(encoded: &mut Vec<u8>, value: U256) {
    encoded.extend_from_slice(&value.to_be_bytes::<32>());
}

fn match_kind_word(match_kind: MatchKind) -> u8 {
    match match_kind {
        MatchKind::Direct => 0,
        MatchKind::MintAssisted => 1,
        MatchKind::MergeAssisted => 2,
    }
}
