# AsceSwap Orderbook Flow Explainer

This document explains how the current Rust orderbook/backend works from the moment a user submits a signed order, how the order is stored, how matching happens, how the demo market maker fits in, and where the onchain contract enters the flow.

## Big Picture

AsceSwap uses a hybrid orderbook design.

Users do not put every order onchain. A user signs an offchain order with their wallet, and the frontend sends that signed order to the backend. The backend validates it, stores it, places it in the in-memory orderbook if it does not match immediately, or creates a match reservation if it does match.

The smart contract is still the final settlement layer. It does not scan the backend orderbook by itself. When the backend has found a match, an executor or relayer must call `matchOrders(...)` on the AsceSwap contract with the matched orders, signatures, and fill amounts.

Simple view:

```text
Wallet / Client
      |
      | POST /orders
      v
HTTP Server
      |
      v
API Service
      |
      v
Validation + Signature Check
      |
      v
Engine
      |
      +--> Matcher
      |
      +--> In-memory Orderbook
      |
      +--> Reservation State
      |
      v
Storage / Postgres
      |
      v
Executor / Relayer
      |
      v
AsceSwap.matchOrders(...)
```

## Main Entry Point

The main user-facing entry point is:

```text
POST /orders
```

The request body is represented by `SubmitOrderRequest`.

It contains:

```text
order
validation
signature_bytes
post_only
rest_on_no_match
reservation_ttl_secs
```

Conceptually, the client sends:

```text
the signed order fields
the user's signature
current validation context
whether the order is post-only
whether the order may rest if it does not match
reservation TTL if it matches
```

Important: these are not all signed fields.

The signed payload is the `order`. The user signs the order intent: maker, market, claim side, amounts, side, expiration, epoch, fee limit, and salt. Fields like `validation`, `post_only`, `rest_on_no_match`, and `reservation_ttl_secs` are request-level instructions or server-side context.

Why the extra request fields exist:

```text
signature_bytes
  The raw wallet signature. It proves order.maker signed this exact order.

validation
  The context used by the backend to decide whether this signed order is still
  usable right now. It includes current time, known filled amount, cancellation
  status, maker epoch, fee rate, and signature status.

post_only
  A maker-safety flag. If true, the order is allowed to rest on the book only.
  If it would immediately take existing liquidity, the backend returns
  PostOnlyWouldCross and does not match it.

rest_on_no_match
  Controls what happens when the incoming order cannot match immediately.
  If true, it becomes an open resting order. If false, it becomes inactive.
  This gives the API support for maker-style orders and taker-only orders.

reservation_ttl_secs
  How long a created match reservation may remain locked while an executor is
  expected to submit it onchain. If the executor does not submit in time, the
  reservation can expire and the liquidity can be released.
```

For a production API, most of `validation` should be derived by the backend/indexer, not blindly trusted from the client. In this testnet code, the request carries it so tests and demos can run deterministically. This is why it may look different from a Polymarket-style client order: the signed order remains clean, but this backend also asks for validation context and matching instructions in the same `POST /orders` request.

The order itself contains:

```text
salt
maker
market_id
claim
maker_amount
taker_amount
side
expiration
epoch
max_fee_rate_bps
```

Important: `side` is from the maker's perspective.

For a `BUY` order:

```text
maker_amount = collateral the maker offers
taker_amount = claim amount the maker wants
max claim amount = taker_amount
```

For a `SELL` order:

```text
maker_amount = claim amount the maker offers
taker_amount = collateral the maker wants
max claim amount = maker_amount
```

## Step-By-Step Order Flow

### 1. User Signs An Order

The frontend builds an order and asks the wallet to sign it. The signature signs the typed order data. The order hash is derived from the exact order fields.

Example user order:

```text
SELL 100 PAYOFF for 40 collateral
```

That means:

```text
claim = PAYOFF
side = SELL
maker_amount = 100
taker_amount = 40
```

The user is saying:

```text
I will give 100 PAYOFF claim tokens.
I want 40 collateral.
```

### 2. Client Sends `POST /orders`

The frontend sends the order and signature bytes to the backend.

The backend first parses the wire/API fields into typed Rust domain objects.

### 3. API Verifies The Signature

If the server is configured with a signature domain, it verifies that:

```text
signature belongs to order.maker
signature signs this exact order
domain chain ID and exchange address match
```

If the signature is invalid, the order is rejected.

### 4. Engine Receives The Order

The API turns the request into a `SubmitOrder` command and sends it to the engine.

The engine:

```text
computes the order hash
checks for duplicate order hash
emits OrderReceived
validates the order
emits OrderValidated if valid
```

Validation checks include:

```text
maker is not zero
market is not zero
amounts are not zero
price is possible
order is not expired
order is not cancelled
maker epoch matches
fee rate is allowed
remaining claim amount is nonzero
signature status is valid if required
```

`price is possible` means the order cannot ask for or offer a price above 1 collateral per claim token. Prediction-market claim tokens settle to at most 1 unit of collateral, so a price above 1 is invalid.

The code checks this in `Order::validate_basic()`:

```text
BUY:
  maker_amount = collateral offered
  taker_amount = claim amount wanted
  valid only if maker_amount <= taker_amount

SELL:
  maker_amount = claim amount offered
  taker_amount = collateral wanted
  valid only if taker_amount <= maker_amount
```

The actual book price is then:

```text
price = collateral_amount / claim_amount
```

encoded as WAD fixed point:

```text
price_wad = collateral_amount * 1e18 / claim_amount
```

### 5. Engine Tries To Match

Before resting the order, the engine asks the matcher:

```text
Can this incoming order match anything already in the book?
```

The incoming order is treated as the taker. Existing resting orders are makers.

This is normal orderbook behavior. When a new order arrives, the engine first checks whether it crosses existing liquidity. If it does, it should execute or reserve against that existing liquidity immediately. Only orders that do not cross should become visible resting liquidity.

The matched order is not lost. It is still stored in `records`, persisted to storage, and connected to a reservation. It simply does not rest in the visible `BTreeMap<Price, VecDeque<OrderHash>>` because it is no longer available liquidity. UI and status APIs should read the order state from `records`/storage, not only from the visible book.

Think of the visible book as "orders available to be matched later". A matched or reserved order is already spoken for, so showing it as open liquidity would be misleading and could allow double matching.

If a match is found:

```text
incoming taker order becomes Reserved
resting maker orders become Reserved
reservation is created
settlement payload is produced
events are emitted
state is persisted
```

If no match is found and `rest_on_no_match = true`:

```text
order is inserted into the orderbook
order state becomes Open
OrderOpened event is emitted
state is persisted
```

If no match is found and `rest_on_no_match = false`:

```text
order becomes Inactive
it does not rest on the book
```

## In-Memory Engine State

The main engine keeps three important kinds of state:

```text
books
records
reservations
```

Visual:

```text
AsceSwapEngine
  |
  +-- books: HashMap<MarketId, MarketOrderBook>
  |
  +-- records: HashMap<OrderHash, OrderRecord>
  |
  +-- reservations: ReservationBook
```

### `books`

`books` stores one `MarketOrderBook` per market.

```text
MarketId -> MarketOrderBook
```

This is the live in-memory liquidity structure used for matching and market depth.

### `records`

`records` stores the lifecycle state for every order the engine knows about.

```text
OrderHash -> OrderRecord
```

An `OrderRecord` tracks:

```text
order hash
full order
signature
state
filled claim amount
whether it is resting
```

This is how the backend knows whether an order is:

```text
Open
Reserved
Submitted
Filled
Cancelled
Inactive
Rejected
```

`Reserved` means the backend has already matched this order with another order, but the match has not been finally settled yet.

In that state:

```text
the order is locked for a specific reservation
the reserved fill amount cannot be matched again
the executor can fetch the settlement payload
if the executor submits the tx, the order moves to Submitted
if settlement succeeds, the order becomes Filled or PartiallyFilled
if the reservation is released or expires, the order is restored to its
available state when possible
```

`Reserved` is therefore an offchain lock. It is not the same as "already filled onchain". The order becomes filled only after the onchain settlement succeeds and the backend commits the reservation.

### `reservations`

`ReservationBook` tracks active matches that have been planned but not fully completed.

Example:

```text
Reservation R
  |
  +-- Taker leg: order B, fill 100
  |
  +-- Maker leg: order A, fill 100
```

Reservations prevent the same liquidity from being matched twice while an execution is pending.

## Orderbook Data Structure

Each `MarketOrderBook` has four separate books:

```text
payoff_bids
payoff_asks
residual_bids
residual_asks
```

This is because each market has two claim sides:

```text
PAYOFF
RESIDUAL
```

and each claim side has two directions:

```text
BUY
SELL
```


Visual:

```text
MarketOrderBook
  |
  +-- PAYOFF bids
  +-- PAYOFF asks
  +-- RESIDUAL bids
  +-- RESIDUAL asks
  |
  +-- orders: HashMap<OrderHash, RestingOrder>
```

The actual price-level structure is:

```text
BTreeMap<Price, VecDeque<OrderHash>>
```

Meaning:

```text
price level -> FIFO queue of order hashes
```

The full order data is stored separately:

```text
orders[order_hash] = RestingOrder
```

So an order is not stored as one big global stack. It is stored by price level and time priority.

Example PAYOFF asks:

```text
PAYOFF asks

0.40 -> [orderA, orderB]
0.45 -> [orderC]
0.50 -> [orderD]
```

For asks, lower price has priority. At the same price, earlier order has priority.

Best ask:

```text
orderA
```

then:

```text
orderB
```

then:

```text
orderC
```

Example PAYOFF bids:

```text
PAYOFF bids

0.60 -> [orderX]
0.55 -> [orderY, orderZ]
0.50 -> [orderW]
```

For bids, higher price has priority. At the same price, earlier order has priority.

Best bid:

```text
orderX
```

then:

```text
orderY
```

then:

```text
orderZ
```

## Matching Rules

The smart contract supports three prediction-market match types.

### Direct Match

Direct match means:

```text
same market
same claim side
opposite order side
```

Examples:

```text
BUY PAYOFF  matches  SELL PAYOFF
SELL PAYOFF matches  BUY PAYOFF
BUY RESIDUAL matches SELL RESIDUAL
SELL RESIDUAL matches BUY RESIDUAL
```

Price condition:

```text
incoming BUY crosses resting SELL when buy price >= sell price
incoming SELL crosses resting BUY when sell price <= buy price
```

Direct match transfers existing claim tokens and collateral between taker and maker.

### Mint-Assisted Match

Mint-assisted match means:

```text
same market
different claim sides
both sides are BUY
```

Example:

```text
User wants to BUY PAYOFF
Maker wants to BUY RESIDUAL
```

Together, they fund a complete prediction-market set. The protocol pulls collateral, mints PAYOFF and RESIDUAL, gives PAYOFF to one trader and RESIDUAL to the other.

This is useful when nobody already owns the claim tokens yet.

### Merge-Assisted Match

Merge-assisted match means:

```text
same market
different claim sides
both sides are SELL
```

Example:

```text
User wants to SELL PAYOFF
Maker wants to SELL RESIDUAL
```

Together, they provide a complete prediction-market set. The protocol merges PAYOFF and RESIDUAL back into collateral, then pays sellers.

This is useful when traders own complementary positions.

## Demo Market Maker Flow

The demo market maker is testnet/demo-only automation.

It is enabled by server environment variables. When enabled, it watches the submit flow internally.

Current demo behavior:

```text
if user order is valid
and user order would rest
and user order is not post-only
and user order was not made by the demo maker
then demo maker creates a signed opposite order
and submits it through the same engine path
```

Example:

```text
User order A:
SELL 100 PAYOFF for 40 collateral
```

The demo maker creates:

```text
Bot order B:
BUY 100 PAYOFF for 40 collateral
```

Then:

```text
A rests first
B is submitted second
B becomes taker
A becomes maker
matcher creates a Direct reservation
```

Visual:

```text
User submits A
      |
      v
A is valid and would rest
      |
      v
Demo maker creates B
      |
      v
Engine submits B
      |
      v
B crosses A
      |
      v
Reservation created
```

Important limitation:

```text
Current demo maker creates direct opposite orders only.
```

That means it follows the contract's Direct match definition:

```text
same claim
opposite side
```

It does not yet generate mint-assisted or merge-assisted orders automatically.

For example:

```text
User BUY PAYOFF
Bot SELL PAYOFF
```

is a Direct match, but it requires the bot to own PAYOFF tokens onchain.

The prediction-market assisted alternative would be:

```text
User BUY PAYOFF
Bot BUY RESIDUAL
```

That is Mint-Assisted. It requires collateral, not pre-existing PAYOFF inventory.

## Who Actually Matches The Orders?

There are two layers:

```text
offchain matching
onchain settlement
```

### Offchain

The Rust backend matches orders.

It scans the in-memory orderbook, finds compatible resting maker orders, creates a match plan, then creates a reservation.

### Onchain

The AsceSwap contract settles the already-planned match.

The executor calls:

```text
matchOrders(
  takerOrder,
  takerSignature,
  makerOrders,
  makerSignatures,
  takerClaimFillAmount,
  makerClaimFillAmounts
)
```

The contract checks:

```text
maker count is valid
signatures are valid
orders are not expired/cancelled
epochs match
fill amounts are valid
match kind is valid
settlement accounting is valid
```

Then it transfers collateral and/or CTF positions according to the match kind.

## Persistence

After every engine update, the API persists:

```text
engine events
latest engine snapshot
orders
reservations
reservation legs
```

The storage abstraction is `EngineStore`.

Conceptual storage shape:

```text
engine_events
  sequence
  created_at
  event_type
  payload

orders
  order_hash
  order fields
  signature_bytes
  order_state
  filled_claim_amount
  resting
  accepted_sequence

reservations
  reservation_id
  status
  created_at
  expires_at

reservation_legs
  reservation_id
  leg_index
  order_hash
  role
  claim_amount

engine_snapshots
  snapshot_id
  next_reservation_sequence
  created_at
  payload
```

The event log is useful for replay/debugging. The snapshot is useful for fast recovery.

On restart:

```text
load latest snapshot
rebuild engine records
rebuild per-market books
rebuild reservation book
continue event sequence
```

## Example Full Demo Flow

Assume demo market maker is enabled with auto-commit.

### Step 1: User submits order A

```text
A = SELL 100 PAYOFF for 40 collateral
```

Events:

```text
OrderReceived(A)
OrderValidated(A)
OrderOpened(A)
```

In memory:

```text
records[A] = Open, resting = true
payoff_asks[0.40].push_back(A)
```

### Step 2: Demo maker creates order B

```text
B = BUY 100 PAYOFF for 40 collateral
```

The bot signs `B` with its configured private key.

### Step 3: Engine submits B

Events:

```text
OrderReceived(B)
OrderValidated(B)
OrderReserved(B)
OrderReserved(A)
ReservationCreated(R)
```

In memory:

```text
records[B] = Reserved
records[A] = Reserved
reservations[R] = [B as taker, A as maker]
```

### Step 4: Auto-commit demo path

If `ASCESWAP_DEMO_MM_AUTO_COMMIT=true`, the backend also performs the mock execution lifecycle:

```text
ReservationSubmitted(R)
OrderSubmitted(B)
OrderSubmitted(A)
ReservationCommitted(R)
OrderFilled(B)
OrderFilled(A)
```

In memory:

```text
records[A] = Filled, resting = false
records[B] = Filled, resting = false
orderbook removes A because fully filled
```

In storage:

```text
events are appended
orders are upserted with latest states
reservation is upserted as committed
snapshot is saved
```

## Things To Remember

The backend orderbook is not the smart contract. It is the offchain coordination layer.

The orderbook stores live liquidity in:

```text
BTreeMap<Price, VecDeque<OrderHash>>
```

The engine stores order lifecycle state in:

```text
HashMap<OrderHash, OrderRecord>
```

The backend creates match reservations. The smart contract settles them.

## Recommended Testnet Executor Flow

Yes, the testnet should have an executor/relayer that drains ready reservations and submits them onchain. That component should be separate from the matcher.

The matcher should not wait every 10 seconds. The matcher should keep accepting orders and creating reservations immediately. The executor should run in the background and poll or subscribe for reservations that are ready to settle.

Good demo architecture:

```text
Matcher / Engine
  |
  | creates Reserved reservation R
  v
Storage
  |
  | executor polls every N seconds or receives event
  v
Executor / Relayer
  |
  | fetches settlement payload
  | simulates matchOrders with eth_call
  | marks reservation Submitted
  | sends transaction
  v
AsceSwap.matchOrders(...)
  |
  | indexer sees success
  v
Backend commits reservation
```

For testnet, a 10 second executor loop is reasonable:

```text
every 10 seconds:
  load Reserved reservations
  skip expired reservations
  fetch settlement payload
  simulate the call
  mark reservation Submitted
  send matchOrders transaction
  after confirmation, commit reservation
  if simulation or tx fails, release the reservation
```

The reservation TTL should be longer than the polling interval plus expected transaction time. For example, if the executor polls every 10 seconds, a TTL like 60-120 seconds is safer than 10 seconds. Otherwise a reservation can expire while the executor is still waiting for RPC, gas estimation, mempool inclusion, or confirmation.

Batching detail:

```text
matchOrders already batches makers for one taker:
  one taker order
  many maker orders
  many maker fill amounts
```

So the first batching level is already inside one reservation: one taker can match against multiple makers in one `matchOrders(...)` call.

If you want to settle many reservations in one chain transaction, that requires either:

```text
a contract multicall/batch executor, or
a relayer contract that loops over multiple matchOrders payloads, or
multiple normal transactions sent by the offchain executor
```

For the current demo, the simplest reliable approach is:

```text
one reservation -> one matchOrders transaction
executor loops every 10 seconds
```

Later, once this is stable, you can add true multi-reservation batching.

The demo market maker currently creates direct matches only. That is valid for demo if the bot account has the right collateral/claim inventory. For a stronger prediction-market demo, the bot should later support assisted matching:

```text
BUY PAYOFF  -> bot BUY RESIDUAL   -> Mint-Assisted
SELL PAYOFF -> bot SELL RESIDUAL  -> Merge-Assisted
```
