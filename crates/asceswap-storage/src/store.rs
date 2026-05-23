use asceswap_engine::{AsceSwapEngine, EngineEvent, EngineSnapshot};
use asceswap_matcher::MatchConfig;

use crate::{
    StorageError, StoredEngineEvent, StoredFill, StoredOrder, StoredReservation, StoredSnapshot,
};

pub trait EngineStore {
    fn put_order(&mut self, order: StoredOrder) -> Result<(), StorageError>;

    fn put_reservation(&mut self, reservation: StoredReservation) -> Result<(), StorageError>;

    fn append_fill(&mut self, fill: StoredFill) -> Result<(), StorageError>;

    fn append_event(&mut self, event: StoredEngineEvent) -> Result<(), StorageError>;

    fn save_snapshot(&mut self, snapshot: StoredSnapshot) -> Result<(), StorageError>;

    fn load_orders(&self) -> Result<Vec<StoredOrder>, StorageError>;

    fn load_reservations(&self) -> Result<Vec<StoredReservation>, StorageError>;

    fn load_fills(&self) -> Result<Vec<StoredFill>, StorageError>;

    fn load_events(&self) -> Result<Vec<StoredEngineEvent>, StorageError>;

    fn load_snapshot(&self) -> Result<Option<StoredSnapshot>, StorageError>;

    fn last_event_sequence(&self) -> Result<Option<u64>, StorageError> {
        Ok(self.load_events()?.last().map(|event| event.sequence))
    }

    fn persist_engine_snapshot(
        &mut self,
        snapshot: EngineSnapshot,
        now: u64,
    ) -> Result<(), StorageError> {
        for order in &snapshot.orders {
            self.put_order(StoredOrder::from_snapshot(order.clone(), now))?;
        }
        for reservation in &snapshot.reservations {
            self.put_reservation(StoredReservation::from_reservation(
                reservation.clone(),
                now,
            ))?;
        }

        self.save_snapshot(StoredSnapshot {
            engine: snapshot,
            created_at: now,
        })
    }

    fn append_engine_events(
        &mut self,
        first_sequence: u64,
        now: u64,
        events: &[EngineEvent],
    ) -> Result<(), StorageError> {
        let mut stored_events = Vec::with_capacity(events.len());
        for (offset, event) in events.iter().enumerate() {
            let sequence = first_sequence
                .checked_add(offset as u64)
                .ok_or(StorageError::SequenceOverflow)?;
            stored_events.push(StoredEngineEvent {
                sequence,
                created_at: now,
                event: event.clone(),
            });
        }

        for event in stored_events {
            self.append_event(event)?;
        }

        Ok(())
    }

    fn persist_engine_update(
        &mut self,
        first_sequence: u64,
        now: u64,
        events: &[EngineEvent],
        snapshot: EngineSnapshot,
    ) -> Result<(), StorageError> {
        self.append_engine_events(first_sequence, now, events)?;
        self.persist_engine_snapshot(snapshot, now)
    }

    fn recover_engine(&self, match_config: MatchConfig) -> Result<AsceSwapEngine, StorageError> {
        let snapshot = self
            .load_snapshot()?
            .ok_or(StorageError::MissingSnapshot)?
            .engine;
        AsceSwapEngine::from_snapshot(match_config, snapshot).map_err(StorageError::Recovery)
    }
}
