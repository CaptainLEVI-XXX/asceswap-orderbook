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

    fn load_orders(&self) -> Vec<StoredOrder>;

    fn load_reservations(&self) -> Vec<StoredReservation>;

    fn load_fills(&self) -> Vec<StoredFill>;

    fn load_events(&self) -> Vec<StoredEngineEvent>;

    fn load_snapshot(&self) -> Option<StoredSnapshot>;

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

    fn recover_engine(&self, match_config: MatchConfig) -> Result<AsceSwapEngine, StorageError> {
        let snapshot = self
            .load_snapshot()
            .ok_or(StorageError::MissingSnapshot)?
            .engine;
        AsceSwapEngine::from_snapshot(match_config, snapshot).map_err(StorageError::Recovery)
    }
}
