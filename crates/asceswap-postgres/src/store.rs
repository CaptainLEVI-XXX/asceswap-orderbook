use std::cell::RefCell;
use std::collections::HashMap;

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
    side, expiration, epoch, max_fee_rate_bps, order_state,
    filled_claim_amount, resting, created_at, updated_at
) VALUES (
    $1, $2::numeric, $3, $4, $5, $6::numeric, $7::numeric,
    $8, $9::numeric, $10::numeric, $11, $12, $13::numeric, $14, $15, $16
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
    order_state = EXCLUDED.order_state,
    filled_claim_amount = EXCLUDED.filled_claim_amount,
    resting = EXCLUDED.resting,
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
    order_state, filled_claim_amount::text, resting, created_at, updated_at
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
    client: RefCell<Client>,
}

impl PostgresEngineStore {
    pub fn connect(params: &str) -> Result<Self, StorageError> {
        let client = Client::connect(params, NoTls).map_err(db_error)?;
        Ok(Self::new(client))
    }

    pub fn new(client: Client) -> Self {
        Self {
            client: RefCell::new(client),
        }
    }

    pub fn run_schema(&mut self) -> Result<(), StorageError> {
        self.client
            .get_mut()
            .batch_execute(POSTGRES_SCHEMA)
            .map_err(db_error)
    }
}

impl EngineStore for PostgresEngineStore {
    fn put_order(&mut self, order: StoredOrder) -> Result<(), StorageError> {
        let mut client = self.client.borrow_mut();
        write_order(&mut *client, order)
    }

    fn put_reservation(&mut self, reservation: StoredReservation) -> Result<(), StorageError> {
        let mut client = self.client.borrow_mut();
        let mut tx = client.transaction().map_err(db_error)?;
        write_reservation(&mut tx, reservation)?;
        tx.commit().map_err(db_error)
    }

    fn append_fill(&mut self, fill: StoredFill) -> Result<(), StorageError> {
        let mut client = self.client.borrow_mut();
        write_fill(&mut *client, fill)
    }

    fn append_event(&mut self, event: StoredEngineEvent) -> Result<(), StorageError> {
        let mut client = self.client.borrow_mut();
        write_event(&mut *client, event)
    }

    fn save_snapshot(&mut self, snapshot: StoredSnapshot) -> Result<(), StorageError> {
        let mut client = self.client.borrow_mut();
        write_snapshot(&mut *client, snapshot)
    }

    fn persist_engine_snapshot(
        &mut self,
        snapshot: EngineSnapshot,
        now: u64,
    ) -> Result<(), StorageError> {
        let mut client = self.client.borrow_mut();
        let mut tx = client.transaction().map_err(db_error)?;
        write_engine_snapshot(&mut tx, snapshot, now)?;
        tx.commit().map_err(db_error)
    }

    fn append_engine_events(
        &mut self,
        first_sequence: u64,
        now: u64,
        events: &[EngineEvent],
    ) -> Result<(), StorageError> {
        let events = stored_events(first_sequence, now, events)?;
        let mut client = self.client.borrow_mut();
        let mut tx = client.transaction().map_err(db_error)?;
        for event in events {
            write_event(&mut tx, event)?;
        }
        tx.commit().map_err(db_error)
    }

    fn persist_engine_update(
        &mut self,
        first_sequence: u64,
        now: u64,
        events: &[EngineEvent],
        snapshot: EngineSnapshot,
    ) -> Result<(), StorageError> {
        let events = stored_events(first_sequence, now, events)?;
        let mut client = self.client.borrow_mut();
        let mut tx = client.transaction().map_err(db_error)?;
        for event in events {
            write_event(&mut tx, event)?;
        }
        write_engine_snapshot(&mut tx, snapshot, now)?;
        tx.commit().map_err(db_error)
    }

    fn load_orders(&self) -> Result<Vec<StoredOrder>, StorageError> {
        let rows = self
            .client
            .borrow_mut()
            .query(SELECT_ORDERS_SQL, &[])
            .map_err(db_error)?;

        rows.into_iter().map(order_from_row).collect()
    }

    fn load_reservations(&self) -> Result<Vec<StoredReservation>, StorageError> {
        let mut client = self.client.borrow_mut();
        let reservation_rows = client
            .query(SELECT_RESERVATIONS_SQL, &[])
            .map_err(db_error)?;
        let leg_rows = client
            .query(SELECT_RESERVATION_LEGS_SQL, &[])
            .map_err(db_error)?;
        drop(client);

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

    fn load_fills(&self) -> Result<Vec<StoredFill>, StorageError> {
        let rows = self
            .client
            .borrow_mut()
            .query(SELECT_FILLS_SQL, &[])
            .map_err(db_error)?;

        rows.into_iter().map(fill_from_row).collect()
    }

    fn load_events(&self) -> Result<Vec<StoredEngineEvent>, StorageError> {
        let rows = self
            .client
            .borrow_mut()
            .query(SELECT_EVENTS_SQL, &[])
            .map_err(db_error)?;

        rows.into_iter().map(event_from_row).collect()
    }

    fn load_snapshot(&self) -> Result<Option<StoredSnapshot>, StorageError> {
        let row = self
            .client
            .borrow_mut()
            .query_opt(SELECT_LATEST_SNAPSHOT_SQL, &[])
            .map_err(db_error)?;
        let Some(row) = row else {
            return Ok(None);
        };

        let next_reservation_sequence =
            i64_to_u64("engine_snapshots.next_reservation_sequence", row.get(0))?;
        let created_at = i64_to_u64("engine_snapshots.created_at", row.get(1))?;
        let orders = self
            .load_orders()?
            .into_iter()
            .map(|stored| stored.snapshot)
            .collect();
        let reservations = self
            .load_reservations()?
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

    fn last_event_sequence(&self) -> Result<Option<u64>, StorageError> {
        let row = self
            .client
            .borrow_mut()
            .query_one("SELECT MAX(sequence) FROM engine_events", &[])
            .map_err(db_error)?;
        row.get::<_, Option<i64>>(0)
            .map(|sequence| i64_to_u64("engine_events.sequence", sequence))
            .transpose()
    }
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
    let order_state = order_state_to_str(snapshot.state);
    let filled_claim_amount = u256_to_string(snapshot.filled_claim_amount);
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
                &order_state,
                &filled_claim_amount,
                &snapshot.resting,
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
    let state = order_state_from_str(row.get::<_, String>(11).as_str())?;
    let filled_claim_amount = u256_from_string(
        "orders.filled_claim_amount",
        row.get::<_, String>(12).as_str(),
    )?;
    let resting = row.get(13);
    let created_at = i64_to_u64("orders.created_at", row.get(14))?;
    let updated_at = i64_to_u64("orders.updated_at", row.get(15))?;

    Ok(StoredOrder {
        snapshot: OrderSnapshot {
            hash,
            order,
            state,
            filled_claim_amount,
            resting,
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
