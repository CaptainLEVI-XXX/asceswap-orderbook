use std::collections::HashMap;

use asceswap_state::ReservationId;
use asceswap_types::OrderHash;

use crate::{
    EngineStore, StorageError, StoredEngineEvent, StoredFill, StoredOrder, StoredReservation,
    StoredSnapshot,
};

#[derive(Clone, Debug, Default)]
pub struct InMemoryEngineStore {
    orders: HashMap<OrderHash, StoredOrder>,
    reservations: HashMap<ReservationId, StoredReservation>,
    fills: HashMap<u64, StoredFill>,
    events: HashMap<u64, StoredEngineEvent>,
    snapshot: Option<StoredSnapshot>,
}

impl InMemoryEngineStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl EngineStore for InMemoryEngineStore {
    fn put_order(&mut self, order: StoredOrder) -> Result<(), StorageError> {
        self.orders.insert(order.hash(), order);
        Ok(())
    }

    fn put_reservation(&mut self, reservation: StoredReservation) -> Result<(), StorageError> {
        self.reservations.insert(reservation.id(), reservation);
        Ok(())
    }

    fn append_fill(&mut self, fill: StoredFill) -> Result<(), StorageError> {
        if self.fills.contains_key(&fill.sequence) {
            return Err(StorageError::DuplicateFillSequence(fill.sequence));
        }

        self.fills.insert(fill.sequence, fill);
        Ok(())
    }

    fn append_event(&mut self, event: StoredEngineEvent) -> Result<(), StorageError> {
        if self.events.contains_key(&event.sequence) {
            return Err(StorageError::DuplicateEventSequence(event.sequence));
        }

        self.events.insert(event.sequence, event);
        Ok(())
    }

    fn save_snapshot(&mut self, snapshot: StoredSnapshot) -> Result<(), StorageError> {
        self.snapshot = Some(snapshot);
        Ok(())
    }

    fn load_orders(&self) -> Result<Vec<StoredOrder>, StorageError> {
        let mut orders = self.orders.values().cloned().collect::<Vec<_>>();
        orders.sort_by(|left, right| {
            left.snapshot
                .hash
                .as_slice()
                .cmp(right.snapshot.hash.as_slice())
        });
        Ok(orders)
    }

    fn load_reservations(&self) -> Result<Vec<StoredReservation>, StorageError> {
        let mut reservations = self.reservations.values().cloned().collect::<Vec<_>>();
        reservations.sort_by(|left, right| {
            left.reservation
                .id
                .as_slice()
                .cmp(right.reservation.id.as_slice())
        });
        Ok(reservations)
    }

    fn load_fills(&self) -> Result<Vec<StoredFill>, StorageError> {
        let mut fills = self.fills.values().cloned().collect::<Vec<_>>();
        fills.sort_by_key(|fill| fill.sequence);
        Ok(fills)
    }

    fn load_events(&self) -> Result<Vec<StoredEngineEvent>, StorageError> {
        let mut events = self.events.values().cloned().collect::<Vec<_>>();
        events.sort_by_key(|event| event.sequence);
        Ok(events)
    }

    fn load_snapshot(&self) -> Result<Option<StoredSnapshot>, StorageError> {
        Ok(self.snapshot.clone())
    }
}
