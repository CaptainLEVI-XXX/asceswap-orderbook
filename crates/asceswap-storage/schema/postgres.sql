-- Phase 4 persistence target for the AsceSwap orderbook backend.
-- Values that mirror Solidity uint256 are stored as NUMERIC(78, 0).
-- Hashes and market ids are 32-byte values; addresses are 20-byte values.

CREATE TABLE IF NOT EXISTS orders (
    order_hash BYTEA PRIMARY KEY CHECK (octet_length(order_hash) = 32),
    salt NUMERIC(78, 0) NOT NULL,
    maker BYTEA NOT NULL CHECK (octet_length(maker) = 20),
    market_id BYTEA NOT NULL CHECK (octet_length(market_id) = 32),
    claim_side SMALLINT NOT NULL CHECK (claim_side IN (0, 1)),
    maker_amount NUMERIC(78, 0) NOT NULL,
    taker_amount NUMERIC(78, 0) NOT NULL,
    side SMALLINT NOT NULL CHECK (side IN (0, 1)),
    expiration NUMERIC(78, 0) NOT NULL,
    epoch NUMERIC(78, 0) NOT NULL,
    max_fee_rate_bps INTEGER NOT NULL CHECK (max_fee_rate_bps BETWEEN 0 AND 65535),
    order_state TEXT NOT NULL,
    filled_claim_amount NUMERIC(78, 0) NOT NULL,
    resting BOOLEAN NOT NULL,
    created_at BIGINT NOT NULL,
    updated_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS orders_market_state_idx ON orders (market_id, order_state);
CREATE INDEX IF NOT EXISTS orders_maker_idx ON orders (maker);

CREATE TABLE IF NOT EXISTS reservations (
    reservation_id BYTEA PRIMARY KEY CHECK (octet_length(reservation_id) = 32),
    status TEXT NOT NULL,
    created_at BIGINT NOT NULL,
    expires_at BIGINT,
    updated_at BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS reservation_legs (
    reservation_id BYTEA NOT NULL REFERENCES reservations (reservation_id),
    leg_index INTEGER NOT NULL CHECK (leg_index >= 0),
    order_hash BYTEA NOT NULL REFERENCES orders (order_hash),
    role SMALLINT NOT NULL CHECK (role IN (0, 1)),
    claim_amount NUMERIC(78, 0) NOT NULL,
    PRIMARY KEY (reservation_id, leg_index)
);

CREATE INDEX IF NOT EXISTS reservation_legs_order_idx ON reservation_legs (order_hash);

CREATE TABLE IF NOT EXISTS fills (
    sequence BIGINT PRIMARY KEY,
    reservation_id BYTEA NOT NULL REFERENCES reservations (reservation_id),
    order_hash BYTEA NOT NULL REFERENCES orders (order_hash),
    claim_amount NUMERIC(78, 0) NOT NULL,
    new_filled_claim_amount NUMERIC(78, 0) NOT NULL,
    created_at BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS fills_order_idx ON fills (order_hash);
CREATE INDEX IF NOT EXISTS fills_reservation_idx ON fills (reservation_id);

CREATE TABLE IF NOT EXISTS engine_events (
    sequence BIGINT PRIMARY KEY,
    created_at BIGINT NOT NULL,
    event_type TEXT NOT NULL,
    payload JSONB NOT NULL
);

CREATE INDEX IF NOT EXISTS engine_events_type_idx ON engine_events (event_type);

CREATE TABLE IF NOT EXISTS engine_snapshots (
    snapshot_id BIGSERIAL PRIMARY KEY,
    next_reservation_sequence BIGINT NOT NULL,
    created_at BIGINT NOT NULL,
    payload JSONB NOT NULL
);
