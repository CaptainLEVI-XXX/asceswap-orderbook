# AsceSwap Orderbook Executor Runbook

This runbook explains the process that settles matched orderbook reservations onchain.

## What This Executor Does

The API and engine decide which signed orders can match. They do not submit a blockchain transaction by themselves.

The executor is the separate process that watches the backend for reserved matches and calls:

```text
AsceSwap.matchOrders(...)
```

High-level flow:

```text
User / market actor submits signed order
        |
        v
POST /orders
        |
        v
Matcher finds compatible liquidity
        |
        v
Backend creates a reserved match
        |
        v
Executor polls /reservations?status=reserved
        |
        v
Executor fetches /reservations/:id/settlement
        |
        v
Executor simulates AsceSwap.matchOrders(...)
        |
        v
Executor sends the transaction
        |
        v
Executor commits or releases the reservation
```

## Component Responsibilities

### Market actor

The market actor is the demo liquidity bot.

It creates the opposite signed order when a testnet user submits an order. Its job is to make sure demo users usually have someone to trade against.

It does not finalize settlement. It is just another order maker.

### Matcher

The matcher lives in the backend engine.

It compares the incoming order against open resting orders. A valid opposite order must have compatible market, claim side, opposite side, and price. If the match is valid, the matcher creates a reservation so those orders cannot be matched twice while settlement is pending.

It does not call the blockchain.

### Executor

The executor is the onchain relayer.

It reads reservations from the backend, converts the settlement payload to the contract ABI shape, simulates `matchOrders`, sends the transaction, waits for the receipt, and updates backend reservation state.

## Required Environment

Run the API server first. The executor talks to that API, not directly to the database.

Required variables:

```bash
export ASCESWAP_API_URL="http://127.0.0.1:8080"
export ASCESWAP_RPC_URL="https://sepolia-rollup.arbitrum.io/rpc"
export ASCESWAP_EXCHANGE_ADDRESS="0x..."
export ASCESWAP_EXECUTOR_PRIVATE_KEY="0x..."
```

Optional variables:

```bash
export ASCESWAP_CHAIN_ID="421614"
export ASCESWAP_EXECUTOR_POLL_SECS="10"
export ASCESWAP_EXECUTOR_RESERVATION_LIMIT="20"
export ASCESWAP_EXECUTOR_CONFIRMATIONS="1"
export ASCESWAP_EXECUTOR_DRY_RUN="false"
export ASCESWAP_EXECUTOR_RELEASE_ON_SIMULATION_FAILURE="false"
```

Do not commit real private keys, RPC secrets, or database URLs. Use the hosting provider's secret manager.

## Running Locally

After the API server is running:

```bash
cargo run -p asceswap-executor
```

For a no-transaction smoke test:

```bash
ASCESWAP_EXECUTOR_DRY_RUN=true cargo run -p asceswap-executor
```

Dry run still fetches reservations and simulates `matchOrders`, but it does not broadcast a transaction.

## Reservation State Handling

For each reserved match, the executor does this:

```text
1. GET /reservations?status=reserved&limit=N
2. GET /reservations/:reservation_id/settlement
3. eth_call AsceSwap.matchOrders(...) for simulation
4. POST /reservations/:reservation_id/submitted
5. Send AsceSwap.matchOrders(...) transaction
6. Wait for configured confirmations
7. If receipt status is success: POST /reservations/:reservation_id/commit
8. If receipt status is failure: POST /reservations/:reservation_id/release
```

By default, simulation failure does not release the reservation automatically. That prevents a bad RPC response or temporary chain issue from unlocking liquidity too aggressively.

For demos, this can be changed with:

```bash
export ASCESWAP_EXECUTOR_RELEASE_ON_SIMULATION_FAILURE=true
```

## Hosting Shape For Testnet

For public testnet usage, run these as separate services:

```text
API server
  Serves POST /orders, read APIs, WebSocket updates

Postgres
  Stores orders, reservations, events, snapshots

Executor
  Polls the API and sends matchOrders transactions

Market actor
  Places demo liquidity orders
```

The executor should run with one funded wallet. That wallet pays gas for settlement transactions.

## Current MVP Limitation

The executor currently logs the transaction hash, but it does not persist an execution job table with tx hashes and retry metadata.

That is acceptable for a controlled testnet demo. Before real public usage, add durable execution tracking so the system can recover cleanly if the executor process restarts after submitting a transaction but before committing the reservation.

