use std::collections::HashMap;
use std::sync::mpsc;
use std::thread;

use asceswap_engine::{EngineEvent, EngineSnapshot, OrderSnapshot};
use asceswap_state::{Reservation, ReservationLeg};
use asceswap_storage::{
    EngineStore, StorageError, StoredEngineEvent, StoredFill, StoredOrder, StoredReservation,
    StoredSnapshot,
};
use asceswap_types::{Order, B256};
use postgres::{Client, GenericClient, NoTls, Row};

use crate::codec::{
    address_from_bytes, address_to_bytes, b256_from_bytes, b256_to_bytes, claim_side_from_i16,
    claim_side_to_i16, decode_event, encode_event, i32_to_u16, i64_to_u64, order_state_from_str,
    order_state_to_str, reservation_leg_role_from_i16, reservation_leg_role_to_i16,
    reservation_status_from_str, reservation_status_to_str, side_from_i16, side_to_i16,
    u256_from_string, u256_to_string, u64_to_i64, usize_to_i32,
};

pub const POSTGRES_SCHEMA: &str = include_str!("../../asceswap-storage/schema/postgres.sql");

const UPSERT_ORDER_SQL: &str = r#"
INSERT INTO orders (
    order_hash, salt, maker, market_id, claim_side, maker_amount, taker_amount,
    side, expiration, epoch, max_fee_rate_bps, signature_bytes, order_state,
    filled_claim_amount, resting, accepted_sequence, created_at, updated_at
) VALUES (
    $1, $2::numeric, $3, $4, $5, $6::numeric, $7::numeric,
    $8, $9::numeric, $10::numeric, $11, $12, $13, $14::numeric, $15, $16, $17, $18
)
ON CONFLICT (order_hash) DO UPDATE SET
    salt = EXCLUDED.salt,
    maker = EXCLUDED.maker,
    market_id = EXCLUDED.market_id,
    claim_side = EXCLUDED.claim_side,
    maker_amount = EXCLUDED.maker_amount,
    taker_amount = EXCLUDED.taker_amount,
    side = EXCLUDED.side,
    expiration = EXCLUDED.expiration,
    epoch = EXCLUDED.epoch,
    max_fee_rate_bps = EXCLUDED.max_fee_rate_bps,
    signature_bytes = EXCLUDED.signature_bytes,
    order_state = EXCLUDED.order_state,
    filled_claim_amount = EXCLUDED.filled_claim_amount,
    resting = EXCLUDED.resting,
    accepted_sequence = EXCLUDED.accepted_sequence,
    created_at = EXCLUDED.created_at,
    updated_at = EXCLUDED.updated_at
"#;

const UPSERT_RESERVATION_SQL: &str = r#"
INSERT INTO reservations (reservation_id, status, created_at, expires_at, updated_at)
VALUES ($1, $2, $3, $4, $5)
ON CONFLICT (reservation_id) DO UPDATE SET
    status = EXCLUDED.status,
    created_at = EXCLUDED.created_at,
    expires_at = EXCLUDED.expires_at,
    updated_at = EXCLUDED.updated_at
"#;

const INSERT_RESERVATION_LEG_SQL: &str = r#"
INSERT INTO reservation_legs (reservation_id, leg_index, order_hash, role, claim_amount)
VALUES ($1, $2, $3, $4, $5::numeric)
"#;

const INSERT_FILL_SQL: &str = r#"
INSERT INTO fills (
    sequence, reservation_id, order_hash, claim_amount, new_filled_claim_amount, created_at
) VALUES ($1, $2, $3, $4::numeric, $5::numeric, $6)
ON CONFLICT (sequence) DO NOTHING
"#;

const INSERT_EVENT_SQL: &str = r#"
INSERT INTO engine_events (sequence, created_at, event_type, payload)
VALUES ($1, $2, $3, $4::jsonb)
ON CONFLICT (sequence) DO NOTHING
"#;

const INSERT_SNAPSHOT_SQL: &str = r#"
INSERT INTO engine_snapshots (next_reservation_sequence, created_at, payload)
VALUES ($1, $2, $3::jsonb)
"#;

const SELECT_ORDERS_SQL: &str = r#"
SELECT
    order_hash, salt::text, maker, market_id, claim_side, maker_amount::text,
    taker_amount::text, side, expiration::text, epoch::text, max_fee_rate_bps,
    signature_bytes, order_state, filled_claim_amount::text, resting, accepted_sequence, created_at, updated_at
FROM orders
ORDER BY order_hash
"#;

const SELECT_RESERVATIONS_SQL: &str = r#"
SELECT reservation_id, status, created_at, expires_at, updated_at
FROM reservations
ORDER BY reservation_id
"#;

const SELECT_RESERVATION_LEGS_SQL: &str = r#"
SELECT reservation_id, order_hash, role, claim_amount::text
FROM reservation_legs
ORDER BY reservation_id, leg_index
"#;

const SELECT_FILLS_SQL: &str = r#"
SELECT sequence, reservation_id, order_hash, claim_amount::text,
    new_filled_claim_amount::text, created_at
FROM fills
ORDER BY sequence
"#;

const SELECT_EVENTS_SQL: &str = r#"
SELECT sequence, created_at, event_type, payload::text
FROM engine_events
ORDER BY sequence
"#;

const SELECT_LATEST_SNAPSHOT_SQL: &str = r#"
SELECT next_reservation_sequence, created_at
FROM engine_snapshots
ORDER BY snapshot_id DESC
LIMIT 1
"#;

pub struct PostgresEngineStore {
    sender: mpsc::Sender<StoreCommand>,
}

impl PostgresEngineStore {
    pub fn connect(params: &str) -> Result<Self, StorageError> {
        let params = params.to_string();
        let (ready_sender, ready_receiver) = mpsc::channel();
        let (sender, receiver) = mpsc::channel();

        thread::spawn(move || {
            let client = Client::connect(&params, NoTls).map_err(db_error);
            match client {
                Ok(client) => {
                    let _ = ready_sender.send(Ok(()));
                    run_store_worker(client, receiver);
                }
                Err(error) => {
                    let _ = ready_sender.send(Err(error));
                }
            }
        });

        ready_receiver
            .recv()
            .map_err(|_| StorageError::backend("postgres worker failed during startup"))??;
        Ok(Self { sender })
    }

    pub fn new(client: Client) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || run_store_worker(client, receiver));
        Self { sender }
    }

    pub fn run_schema(&mut self) -> Result<(), StorageError> {
        self.request(StoreCommand::RunSchema)
    }

    fn request<T: Send + 'static>(
        &self,
        build: impl FnOnce(mpsc::Sender<Result<T, StorageError>>) -> StoreCommand,
    ) -> Result<T, StorageError> {
        let (respond_to, response) = mpsc::channel();
        self.sender
            .send(build(respond_to))
            .map_err(|_| StorageError::backend("postgres worker stopped"))?;
        response
            .recv()
            .map_err(|_| StorageError::backend("postgres worker stopped"))?
    }
}

impl EngineStore for PostgresEngineStore {
    fn put_order(&mut self, order: StoredOrder) -> Result<(), StorageError> {
        self.request(|respond_to| StoreCommand::PutOrder { order, respond_to })
    }

    fn put_reservation(&mut self, reservation: StoredReservation) -> Result<(), StorageError> {
        self.request(|respond_to| StoreCommand::PutReservation {
            reservation,
            respond_to,
        })
    }

    fn append_fill(&mut self, fill: StoredFill) -> Result<(), StorageError> {
        self.request(|respond_to| StoreCommand::AppendFill { fill, respond_to })
    }

    fn append_event(&mut self, event: StoredEngineEvent) -> Result<(), StorageError> {
        self.request(|respond_to| StoreCommand::AppendEvent { event, respond_to })
    }

    fn save_snapshot(&mut self, snapshot: StoredSnapshot) -> Result<(), StorageError> {
        self.request(|respond_to| StoreCommand::SaveSnapshot {
            snapshot,
            respond_to,
        })
    }

    fn persist_engine_snapshot(
        &mut self,
        snapshot: EngineSnapshot,
        now: u64,
    ) -> Result<(), StorageError> {
        self.request(|respond_to| StoreCommand::PersistEngineSnapshot {
            snapshot,
            now,
            respond_to,
        })
    }

    fn append_engine_events(
        &mut self,
        first_sequence: u64,
        now: u64,
        events: &[EngineEvent],
    ) -> Result<(), StorageError> {
        let events = stored_events(first_sequence, now, events)?;
        self.request(|respond_to| StoreCommand::AppendStoredEvents { events, respond_to })
    }

    fn persist_engine_update(
        &mut self,
        first_sequence: u64,
        now: u64,
        events: &[EngineEvent],
        snapshot: EngineSnapshot,
    ) -> Result<(), StorageError> {
        let events = stored_events(first_sequence, now, events)?;
        self.request(|respond_to| StoreCommand::PersistEngineUpdate {
            events,
            snapshot,
            now,
            respond_to,
        })
    }

    fn load_orders(&self) -> Result<Vec<StoredOrder>, StorageError> {
        self.request(StoreCommand::LoadOrders)
    }

    fn load_reservations(&self) -> Result<Vec<StoredReservation>, StorageError> {
        self.request(StoreCommand::LoadReservations)
    }

    fn load_fills(&self) -> Result<Vec<StoredFill>, StorageError> {
        self.request(StoreCommand::LoadFills)
    }

    fn load_events(&self) -> Result<Vec<StoredEngineEvent>, StorageError> {
        self.request(StoreCommand::LoadEvents)
    }

    fn load_snapshot(&self) -> Result<Option<StoredSnapshot>, StorageError> {
        self.request(StoreCommand::LoadSnapshot)
    }

    fn last_event_sequence(&self) -> Result<Option<u64>, StorageError> {
        self.request(StoreCommand::LastEventSequence)
    }
}

enum StoreCommand {
    RunSchema(mpsc::Sender<Result<(), StorageError>>),
    PutOrder {
        order: StoredOrder,
        respond_to: mpsc::Sender<Result<(), StorageError>>,
    },
    PutReservation {
        reservation: StoredReservation,
        respond_to: mpsc::Sender<Result<(), StorageError>>,
    },
    AppendFill {
        fill: StoredFill,
        respond_to: mpsc::Sender<Result<(), StorageError>>,
    },
    AppendEvent {
        event: StoredEngineEvent,
        respond_to: mpsc::Sender<Result<(), StorageError>>,
    },
    SaveSnapshot {
        snapshot: StoredSnapshot,
        respond_to: mpsc::Sender<Result<(), StorageError>>,
    },
    PersistEngineSnapshot {
        snapshot: EngineSnapshot,
        now: u64,
        respond_to: mpsc::Sender<Result<(), StorageError>>,
    },
    AppendStoredEvents {
        events: Vec<StoredEngineEvent>,
        respond_to: mpsc::Sender<Result<(), StorageError>>,
    },
    PersistEngineUpdate {
        events: Vec<StoredEngineEvent>,
        snapshot: EngineSnapshot,
        now: u64,
        respond_to: mpsc::Sender<Result<(), StorageError>>,
    },
    LoadOrders(mpsc::Sender<Result<Vec<StoredOrder>, StorageError>>),
    LoadReservations(mpsc::Sender<Result<Vec<StoredReservation>, StorageError>>),
    LoadFills(mpsc::Sender<Result<Vec<StoredFill>, StorageError>>),
    LoadEvents(mpsc::Sender<Result<Vec<StoredEngineEvent>, StorageError>>),
    LoadSnapshot(mpsc::Sender<Result<Option<StoredSnapshot>, StorageError>>),
    LastEventSequence(mpsc::Sender<Result<Option<u64>, StorageError>>),
}

fn run_store_worker(mut client: Client, receiver: mpsc::Receiver<StoreCommand>) {
    while let Ok(command) = receiver.recv() {
        match command {
            StoreCommand::RunSchema(respond_to) => {
                let _ = respond_to.send(client.batch_execute(POSTGRES_SCHEMA).map_err(db_error));
            }
            StoreCommand::PutOrder { order, respond_to } => {
                let _ = respond_to.send(write_order(&mut client, order));
            }
            StoreCommand::PutReservation {
                reservation,
                respond_to,
            } => {
                let _ = respond_to.send(write_reservation_transaction(&mut client, reservation));
            }
            StoreCommand::AppendFill { fill, respond_to } => {
                let _ = respond_to.send(write_fill(&mut client, fill));
            }
            StoreCommand::AppendEvent { event, respond_to } => {
                let _ = respond_to.send(write_event(&mut client, event));
            }
            StoreCommand::SaveSnapshot {
                snapshot,
                respond_to,
            } => {
                let _ = respond_to.send(write_snapshot(&mut client, snapshot));
            }
            StoreCommand::PersistEngineSnapshot {
                snapshot,
                now,
                respond_to,
            } => {
                let _ = respond_to.send(write_engine_snapshot_transaction(
                    &mut client,
                    snapshot,
                    now,
                ));
            }
            StoreCommand::AppendStoredEvents { events, respond_to } => {
                let _ = respond_to.send(write_events_transaction(&mut client, events));
            }
            StoreCommand::PersistEngineUpdate {
                events,
                snapshot,
                now,
                respond_to,
            } => {
                let _ = respond_to.send(write_engine_update_transaction(
                    &mut client,
                    events,
                    snapshot,
                    now,
                ));
            }
            StoreCommand::LoadOrders(respond_to) => {
                let _ = respond_to.send(load_orders_from_client(&mut client));
            }
            StoreCommand::LoadReservations(respond_to) => {
                let _ = respond_to.send(load_reservations_from_client(&mut client));
            }
            StoreCommand::LoadFills(respond_to) => {
                let _ = respond_to.send(load_fills_from_client(&mut client));
            }
            StoreCommand::LoadEvents(respond_to) => {
                let _ = respond_to.send(load_events_from_client(&mut client));
            }
            StoreCommand::LoadSnapshot(respond_to) => {
                let _ = respond_to.send(load_snapshot_from_client(&mut client));
            }
            StoreCommand::LastEventSequence(respond_to) => {
                let _ = respond_to.send(last_event_sequence_from_client(&mut client));
            }
        }
    }
}

fn write_reservation_transaction(
    client: &mut Client,
    reservation: StoredReservation,
) -> Result<(), StorageError> {
    let mut tx = client.transaction().map_err(db_error)?;
    write_reservation(&mut tx, reservation)?;
    tx.commit().map_err(db_error)
}

fn write_engine_snapshot_transaction(
    client: &mut Client,
    snapshot: EngineSnapshot,
    now: u64,
) -> Result<(), StorageError> {
    let mut tx = client.transaction().map_err(db_error)?;
    write_engine_snapshot(&mut tx, snapshot, now)?;
    tx.commit().map_err(db_error)
}

fn write_events_transaction(
    client: &mut Client,
    events: Vec<StoredEngineEvent>,
) -> Result<(), StorageError> {
    let mut tx = client.transaction().map_err(db_error)?;
    for event in events {
        write_event(&mut tx, event)?;
    }
    tx.commit().map_err(db_error)
}

fn write_engine_update_transaction(
    client: &mut Client,
    events: Vec<StoredEngineEvent>,
    snapshot: EngineSnapshot,
    now: u64,
) -> Result<(), StorageError> {
    let mut tx = client.transaction().map_err(db_error)?;
    for event in events {
        write_event(&mut tx, event)?;
    }
    write_engine_snapshot(&mut tx, snapshot, now)?;
    tx.commit().map_err(db_error)
}

fn load_orders_from_client(client: &mut Client) -> Result<Vec<StoredOrder>, StorageError> {
    let rows = client.query(SELECT_ORDERS_SQL, &[]).map_err(db_error)?;
    rows.into_iter().map(order_from_row).collect()
}

fn load_reservations_from_client(
    client: &mut Client,
) -> Result<Vec<StoredReservation>, StorageError> {
    let reservation_rows = client
        .query(SELECT_RESERVATIONS_SQL, &[])
        .map_err(db_error)?;
    let leg_rows = client
        .query(SELECT_RESERVATION_LEGS_SQL, &[])
        .map_err(db_error)?;

    let mut legs_by_reservation = HashMap::<B256, Vec<ReservationLeg>>::new();
    for row in leg_rows {
        let reservation_id = b256_from_bytes("reservation_legs.reservation_id", row.get(0))?;
        let order_hash = b256_from_bytes("reservation_legs.order_hash", row.get(1))?;
        let role = reservation_leg_role_from_i16("reservation_legs.role", row.get(2))?;
        let claim_amount = u256_from_string(
            "reservation_legs.claim_amount",
            row.get::<_, String>(3).as_str(),
        )?;
        legs_by_reservation
            .entry(reservation_id)
            .or_default()
            .push(ReservationLeg {
                order_hash,
                role,
                claim_amount,
            });
    }

    let mut reservations = Vec::with_capacity(reservation_rows.len());
    for row in reservation_rows {
        let id = b256_from_bytes("reservations.reservation_id", row.get(0))?;
        let status = reservation_status_from_str(row.get::<_, String>(1).as_str())?;
        let created_at = i64_to_u64("reservations.created_at", row.get(2))?;
        let expires_at = row
            .get::<_, Option<i64>>(3)
            .map(|value| i64_to_u64("reservations.expires_at", value))
            .transpose()?;
        let updated_at = i64_to_u64("reservations.updated_at", row.get(4))?;
        let legs = legs_by_reservation.remove(&id).unwrap_or_default();

        reservations.push(StoredReservation {
            reservation: Reservation {
                id,
                status,
                created_at,
                expires_at,
                legs,
            },
            created_at,
            updated_at,
        });
    }

    Ok(reservations)
}

fn load_fills_from_client(client: &mut Client) -> Result<Vec<StoredFill>, StorageError> {
    let rows = client.query(SELECT_FILLS_SQL, &[]).map_err(db_error)?;
    rows.into_iter().map(fill_from_row).collect()
}

fn load_events_from_client(client: &mut Client) -> Result<Vec<StoredEngineEvent>, StorageError> {
    let rows = client.query(SELECT_EVENTS_SQL, &[]).map_err(db_error)?;
    rows.into_iter().map(event_from_row).collect()
}

fn load_snapshot_from_client(client: &mut Client) -> Result<Option<StoredSnapshot>, StorageError> {
    let row = client
        .query_opt(SELECT_LATEST_SNAPSHOT_SQL, &[])
        .map_err(db_error)?;
    let Some(row) = row else {
        return Ok(None);
    };

    let next_reservation_sequence =
        i64_to_u64("engine_snapshots.next_reservation_sequence", row.get(0))?;
    let created_at = i64_to_u64("engine_snapshots.created_at", row.get(1))?;
    let orders = load_orders_from_client(client)?
        .into_iter()
        .map(|stored| stored.snapshot)
        .collect();
    let reservations = load_reservations_from_client(client)?
        .into_iter()
        .map(|stored| stored.reservation)
        .collect();

    Ok(Some(StoredSnapshot {
        engine: EngineSnapshot {
            orders,
            reservations,
            next_reservation_sequence,
        },
        created_at,
    }))
}

fn last_event_sequence_from_client(client: &mut Client) -> Result<Option<u64>, StorageError> {
    let row = client
        .query_one("SELECT MAX(sequence) FROM engine_events", &[])
        .map_err(db_error)?;
    row.get::<_, Option<i64>>(0)
        .map(|sequence| i64_to_u64("engine_events.sequence", sequence))
        .transpose()
}

fn write_engine_snapshot(
    client: &mut impl GenericClient,
    snapshot: EngineSnapshot,
    now: u64,
) -> Result<(), StorageError> {
    for order in &snapshot.orders {
        write_order(client, StoredOrder::from_snapshot(order.clone(), now))?;
    }
    for reservation in &snapshot.reservations {
        write_reservation(
            client,
            StoredReservation::from_reservation(reservation.clone(), now),
        )?;
    }

    write_snapshot(
        client,
        StoredSnapshot {
            engine: snapshot,
            created_at: now,
        },
    )
}

fn stored_events(
    first_sequence: u64,
    now: u64,
    events: &[EngineEvent],
) -> Result<Vec<StoredEngineEvent>, StorageError> {
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

    Ok(stored_events)
}

fn write_order(client: &mut impl GenericClient, order: StoredOrder) -> Result<(), StorageError> {
    let snapshot = order.snapshot;
    let inner = snapshot.order;
    let order_hash = b256_to_bytes(snapshot.hash);
    let salt = u256_to_string(inner.salt);
    let maker = address_to_bytes(inner.maker);
    let market_id = b256_to_bytes(inner.market_id);
    let claim_side = claim_side_to_i16(inner.claim);
    let maker_amount = u256_to_string(inner.maker_amount);
    let taker_amount = u256_to_string(inner.taker_amount);
    let side = side_to_i16(inner.side);
    let expiration = u256_to_string(inner.expiration);
    let epoch = u256_to_string(inner.epoch);
    let max_fee_rate_bps = i32::from(inner.max_fee_rate_bps);
    let signature = snapshot.signature;
    let order_state = order_state_to_str(snapshot.state);
    let filled_claim_amount = u256_to_string(snapshot.filled_claim_amount);
    let accepted_sequence = snapshot
        .accepted_sequence
        .map(|sequence| u64_to_i64("order.accepted_sequence", sequence))
        .transpose()?;
    let created_at = u64_to_i64("order.created_at", order.created_at)?;
    let updated_at = u64_to_i64("order.updated_at", order.updated_at)?;

    client
        .execute(
            UPSERT_ORDER_SQL,
            &[
                &order_hash,
                &salt,
                &maker,
                &market_id,
                &claim_side,
                &maker_amount,
                &taker_amount,
                &side,
                &expiration,
                &epoch,
                &max_fee_rate_bps,
                &signature,
                &order_state,
                &filled_claim_amount,
                &snapshot.resting,
                &accepted_sequence,
                &created_at,
                &updated_at,
            ],
        )
        .map_err(db_error)?;

    Ok(())
}

fn write_reservation(
    client: &mut impl GenericClient,
    reservation: StoredReservation,
) -> Result<(), StorageError> {
    let inner = reservation.reservation;
    let reservation_id = b256_to_bytes(inner.id);
    let status = reservation_status_to_str(inner.status);
    let created_at = u64_to_i64("reservation.created_at", inner.created_at)?;
    let expires_at = match inner.expires_at {
        Some(value) => Some(u64_to_i64("reservation.expires_at", value)?),
        None => None,
    };
    let updated_at = u64_to_i64("reservation.updated_at", reservation.updated_at)?;

    client
        .execute(
            UPSERT_RESERVATION_SQL,
            &[
                &reservation_id,
                &status,
                &created_at,
                &expires_at,
                &updated_at,
            ],
        )
        .map_err(db_error)?;
    client
        .execute(
            "DELETE FROM reservation_legs WHERE reservation_id = $1",
            &[&reservation_id],
        )
        .map_err(db_error)?;

    for (index, leg) in inner.legs.iter().enumerate() {
        let leg_index = usize_to_i32("reservation.leg_index", index)?;
        let order_hash = b256_to_bytes(leg.order_hash);
        let role = reservation_leg_role_to_i16(leg.role);
        let claim_amount = u256_to_string(leg.claim_amount);
        client
            .execute(
                INSERT_RESERVATION_LEG_SQL,
                &[
                    &reservation_id,
                    &leg_index,
                    &order_hash,
                    &role,
                    &claim_amount,
                ],
            )
            .map_err(db_error)?;
    }

    Ok(())
}

fn write_fill(client: &mut impl GenericClient, fill: StoredFill) -> Result<(), StorageError> {
    let sequence = u64_to_i64("fill.sequence", fill.sequence)?;
    let reservation_id = b256_to_bytes(fill.reservation_id);
    let order_hash = b256_to_bytes(fill.order_hash);
    let claim_amount = u256_to_string(fill.claim_amount);
    let new_filled_claim_amount = u256_to_string(fill.new_filled_claim_amount);
    let created_at = u64_to_i64("fill.created_at", fill.created_at)?;

    let written = client
        .execute(
            INSERT_FILL_SQL,
            &[
                &sequence,
                &reservation_id,
                &order_hash,
                &claim_amount,
                &new_filled_claim_amount,
                &created_at,
            ],
        )
        .map_err(db_error)?;
    if written == 0 {
        return Err(StorageError::DuplicateFillSequence(fill.sequence));
    }

    Ok(())
}

fn write_event(
    client: &mut impl GenericClient,
    event: StoredEngineEvent,
) -> Result<(), StorageError> {
    let sequence = u64_to_i64("event.sequence", event.sequence)?;
    let created_at = u64_to_i64("event.created_at", event.created_at)?;
    let encoded = encode_event(&event.event);

    let written = client
        .execute(
            INSERT_EVENT_SQL,
            &[&sequence, &created_at, &encoded.kind, &encoded.payload],
        )
        .map_err(db_error)?;
    if written == 0 {
        return Err(StorageError::DuplicateEventSequence(event.sequence));
    }

    Ok(())
}

fn write_snapshot(
    client: &mut impl GenericClient,
    snapshot: StoredSnapshot,
) -> Result<(), StorageError> {
    let next_reservation_sequence = u64_to_i64(
        "snapshot.next_reservation_sequence",
        snapshot.engine.next_reservation_sequence,
    )?;
    let created_at = u64_to_i64("snapshot.created_at", snapshot.created_at)?;
    let payload = serde_json::json!({
        "source": "normalized_tables",
        "order_count": snapshot.engine.orders.len(),
        "reservation_count": snapshot.engine.reservations.len(),
    })
    .to_string();

    client
        .execute(
            INSERT_SNAPSHOT_SQL,
            &[&next_reservation_sequence, &created_at, &payload],
        )
        .map_err(db_error)?;

    Ok(())
}

fn order_from_row(row: Row) -> Result<StoredOrder, StorageError> {
    let hash = b256_from_bytes("orders.order_hash", row.get(0))?;
    let order = Order {
        salt: u256_from_string("orders.salt", row.get::<_, String>(1).as_str())?,
        maker: address_from_bytes("orders.maker", row.get(2))?,
        market_id: b256_from_bytes("orders.market_id", row.get(3))?,
        claim: claim_side_from_i16("orders.claim_side", row.get(4))?,
        maker_amount: u256_from_string("orders.maker_amount", row.get::<_, String>(5).as_str())?,
        taker_amount: u256_from_string("orders.taker_amount", row.get::<_, String>(6).as_str())?,
        side: side_from_i16("orders.side", row.get(7))?,
        expiration: u256_from_string("orders.expiration", row.get::<_, String>(8).as_str())?,
        epoch: u256_from_string("orders.epoch", row.get::<_, String>(9).as_str())?,
        max_fee_rate_bps: i32_to_u16("orders.max_fee_rate_bps", row.get(10))?,
    };
    let signature = row.get::<_, Option<Vec<u8>>>(11);
    let state = order_state_from_str(row.get::<_, String>(12).as_str())?;
    let filled_claim_amount = u256_from_string(
        "orders.filled_claim_amount",
        row.get::<_, String>(13).as_str(),
    )?;
    let resting = row.get(14);
    let accepted_sequence = row
        .get::<_, Option<i64>>(15)
        .map(|sequence| i64_to_u64("orders.accepted_sequence", sequence))
        .transpose()?;
    let created_at = i64_to_u64("orders.created_at", row.get(16))?;
    let updated_at = i64_to_u64("orders.updated_at", row.get(17))?;

    Ok(StoredOrder {
        snapshot: OrderSnapshot {
            hash,
            order,
            signature,
            state,
            filled_claim_amount,
            resting,
            accepted_sequence,
        },
        created_at,
        updated_at,
    })
}

fn fill_from_row(row: Row) -> Result<StoredFill, StorageError> {
    Ok(StoredFill {
        sequence: i64_to_u64("fills.sequence", row.get(0))?,
        reservation_id: b256_from_bytes("fills.reservation_id", row.get(1))?,
        order_hash: b256_from_bytes("fills.order_hash", row.get(2))?,
        claim_amount: u256_from_string("fills.claim_amount", row.get::<_, String>(3).as_str())?,
        new_filled_claim_amount: u256_from_string(
            "fills.new_filled_claim_amount",
            row.get::<_, String>(4).as_str(),
        )?,
        created_at: i64_to_u64("fills.created_at", row.get(5))?,
    })
}

fn event_from_row(row: Row) -> Result<StoredEngineEvent, StorageError> {
    let event_type = row.get::<_, String>(2);
    let payload = row.get::<_, String>(3);
    Ok(StoredEngineEvent {
        sequence: i64_to_u64("engine_events.sequence", row.get(0))?,
        created_at: i64_to_u64("engine_events.created_at", row.get(1))?,
        event: decode_event(&event_type, &payload)?,
    })
}

fn db_error(error: postgres::Error) -> StorageError {
    StorageError::backend(error)
}
