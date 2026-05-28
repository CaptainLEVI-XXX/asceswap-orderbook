use asceswap_engine::{
    AsceSwapEngine, EngineSnapshot, OrderSnapshot, SubmitOrder, SubmitOrderOutcome,
};
use asceswap_matcher::MatchConfig;
use asceswap_state::{
    OrderState, Reservation, ReservationLeg, ReservationLegRole, ReservationStatus,
};
use asceswap_types::{Address, ClaimSide, MarketId, Order, Side, B256, U256};
use asceswap_validation::{order_hash, OrderValidationContext, SignatureCheck};

use crate::{spawn_market_actor, MarketActorError, MarketActorRouter};

fn market_id() -> MarketId {
    B256::repeat_byte(7)
}

fn other_market_id() -> MarketId {
    B256::repeat_byte(8)
}

fn sell_order(
    salt: u64,
    market_id: MarketId,
    maker: u8,
    claim_amount: u64,
    collateral_amount: u64,
) -> Order {
    Order {
        salt: U256::from(salt),
        maker: Address::repeat_byte(maker),
        market_id,
        claim: ClaimSide::Payoff,
        maker_amount: U256::from(claim_amount),
        taker_amount: U256::from(collateral_amount),
        side: Side::Sell,
        expiration: U256::ZERO,
        epoch: U256::from(1),
        max_fee_rate_bps: 100,
    }
}

fn buy_order(
    salt: u64,
    market_id: MarketId,
    maker: u8,
    claim_amount: u64,
    collateral_amount: u64,
) -> Order {
    Order {
        salt: U256::from(salt),
        maker: Address::repeat_byte(maker),
        market_id,
        claim: ClaimSide::Payoff,
        maker_amount: U256::from(collateral_amount),
        taker_amount: U256::from(claim_amount),
        side: Side::Buy,
        expiration: U256::ZERO,
        epoch: U256::from(1),
        max_fee_rate_bps: 100,
    }
}

fn validation(order: &Order, now: u64) -> OrderValidationContext {
    OrderValidationContext::new(now)
        .with_expected_order_hash(order_hash(order))
        .with_maker_epoch(order.epoch)
        .with_fee_rate_bps(0)
        .with_signature(SignatureCheck::Valid)
        .with_required_signature(true)
}

fn submit(order: Order, now: u64) -> SubmitOrder {
    SubmitOrder::new(order.clone(), validation(&order, now))
}

#[tokio::test]
async fn actor_rests_order_and_returns_market_depth() {
    let handle = spawn_market_actor(market_id(), AsceSwapEngine::default(), 8).unwrap();
    let order = sell_order(1, market_id(), 1, 100, 40);
    let order_hash = order_hash(&order);

    let result = handle.submit_order(submit(order, 100)).await.unwrap();

    assert!(matches!(result.outcome, SubmitOrderOutcome::Rested { .. }));
    assert_eq!(
        handle
            .order_record(order_hash)
            .await
            .unwrap()
            .unwrap()
            .state(),
        OrderState::Open
    );
    let depth = handle.depth(ClaimSide::Payoff, Side::Sell).await.unwrap();
    assert_eq!(depth.len(), 1);
    assert_eq!(depth[0].total_claim_amount, U256::from(100));
}

#[tokio::test]
async fn actor_rejects_cross_market_submit_without_mutating_state() {
    let handle = spawn_market_actor(market_id(), AsceSwapEngine::default(), 8).unwrap();
    let order = sell_order(1, other_market_id(), 1, 100, 40);

    let error = handle.submit_order(submit(order, 100)).await.unwrap_err();

    assert_eq!(
        error,
        MarketActorError::WrongMarket {
            expected: market_id(),
            actual: other_market_id(),
        }
    );
    assert!(handle.snapshot().await.unwrap().orders.is_empty());
}

#[tokio::test]
async fn actor_serializes_match_and_reservation_release() {
    let handle = spawn_market_actor(market_id(), AsceSwapEngine::default(), 8).unwrap();
    let maker = sell_order(1, market_id(), 1, 100, 40);
    let maker_hash = order_hash(&maker);
    handle.submit_order(submit(maker, 100)).await.unwrap();

    let taker = buy_order(2, market_id(), 2, 100, 50);
    let result = handle.submit_order(submit(taker, 101)).await.unwrap();
    let reservation_id = match result.outcome {
        SubmitOrderOutcome::Matched { reservation_id, .. } => reservation_id,
        other => panic!("expected matched outcome, got {other:?}"),
    };

    assert_eq!(
        handle
            .reservation(reservation_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ReservationStatus::Reserved
    );
    handle.release_reservation(reservation_id).await.unwrap();

    assert_eq!(
        handle
            .order_record(maker_hash)
            .await
            .unwrap()
            .unwrap()
            .state(),
        OrderState::Open
    );
}

#[test]
fn actor_rejects_engine_snapshot_with_other_market_state() {
    let mut engine = AsceSwapEngine::default();
    let order = sell_order(1, other_market_id(), 1, 100, 40);
    engine.submit_order(submit(order, 100)).unwrap();

    let error = spawn_market_actor(market_id(), engine, 8).unwrap_err();

    assert_eq!(
        error,
        MarketActorError::WrongMarket {
            expected: market_id(),
            actual: other_market_id(),
        }
    );
}

#[test]
fn actor_requires_bounded_nonzero_inbox() {
    let error = spawn_market_actor(market_id(), AsceSwapEngine::default(), 0).unwrap_err();

    assert_eq!(error, MarketActorError::InboxCapacityZero);
}

#[tokio::test]
async fn router_dispatches_depth_by_market() {
    let mut router = MarketActorRouter::new();
    router
        .spawn_market(market_id(), AsceSwapEngine::default(), 8)
        .unwrap();
    router
        .spawn_market(other_market_id(), AsceSwapEngine::default(), 8)
        .unwrap();

    let first = sell_order(1, market_id(), 1, 100, 40);
    let second = sell_order(2, other_market_id(), 2, 200, 80);
    router.submit_order(submit(first, 100)).await.unwrap();
    router.submit_order(submit(second, 100)).await.unwrap();

    let first_depth = router
        .depth(market_id(), ClaimSide::Payoff, Side::Sell)
        .await
        .unwrap();
    let second_depth = router
        .depth(other_market_id(), ClaimSide::Payoff, Side::Sell)
        .await
        .unwrap();

    assert_eq!(first_depth[0].total_claim_amount, U256::from(100));
    assert_eq!(second_depth[0].total_claim_amount, U256::from(200));
}

#[tokio::test]
async fn router_routes_cancel_by_order_hash() {
    let mut router = MarketActorRouter::new();
    router
        .spawn_market(market_id(), AsceSwapEngine::default(), 8)
        .unwrap();
    let order = sell_order(1, market_id(), 1, 100, 40);
    let order_hash = order_hash(&order);
    router.submit_order(submit(order, 100)).await.unwrap();

    router
        .cancel_order(asceswap_engine::CancelOrder::new(order_hash))
        .await
        .unwrap();

    assert_eq!(
        router
            .order_record(order_hash)
            .await
            .unwrap()
            .unwrap()
            .state(),
        OrderState::Cancelled
    );
}

#[tokio::test]
async fn router_routes_reservation_updates() {
    let mut router = MarketActorRouter::new();
    router
        .spawn_market(market_id(), AsceSwapEngine::default(), 8)
        .unwrap();
    let maker = sell_order(1, market_id(), 1, 100, 40);
    router.submit_order(submit(maker, 100)).await.unwrap();

    let taker = buy_order(2, market_id(), 2, 100, 50);
    let result = router.submit_order(submit(taker, 101)).await.unwrap();
    let reservation_id = match result.outcome {
        SubmitOrderOutcome::Matched { reservation_id, .. } => reservation_id,
        other => panic!("expected matched outcome, got {other:?}"),
    };

    router.release_reservation(reservation_id).await.unwrap();

    assert_eq!(
        router
            .reservation(reservation_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ReservationStatus::Released
    );
}

#[tokio::test]
async fn router_rejects_duplicate_market() {
    let mut router = MarketActorRouter::new();
    router
        .spawn_market(market_id(), AsceSwapEngine::default(), 8)
        .unwrap();

    let error = router
        .spawn_market(market_id(), AsceSwapEngine::default(), 8)
        .unwrap_err();

    assert_eq!(error, MarketActorError::DuplicateMarket(market_id()));
}

#[tokio::test]
async fn router_indexes_recovered_snapshot_orders() {
    let mut engine = AsceSwapEngine::default();
    let order = sell_order(1, market_id(), 1, 100, 40);
    let order_hash = order_hash(&order);
    engine.submit_order(submit(order, 100)).unwrap();

    let mut router = MarketActorRouter::new();
    router.spawn_market(market_id(), engine, 8).unwrap();

    assert_eq!(
        router
            .order_record(order_hash)
            .await
            .unwrap()
            .unwrap()
            .state(),
        OrderState::Open
    );
}

#[tokio::test]
async fn router_rejects_unknown_routes() {
    let mut router = MarketActorRouter::new();
    router
        .spawn_market(market_id(), AsceSwapEngine::default(), 8)
        .unwrap();
    let unknown_order = B256::repeat_byte(99);
    let unknown_reservation = B256::repeat_byte(100);

    assert_eq!(
        router
            .cancel_order(asceswap_engine::CancelOrder::new(unknown_order))
            .await
            .unwrap_err(),
        MarketActorError::MissingOrderRoute(unknown_order)
    );
    assert_eq!(
        router
            .release_reservation(unknown_reservation)
            .await
            .unwrap_err(),
        MarketActorError::MissingReservationRoute(unknown_reservation)
    );
}

#[tokio::test]
async fn router_merges_global_snapshot_deterministically() {
    let mut router = MarketActorRouter::new();
    router
        .spawn_market(other_market_id(), AsceSwapEngine::default(), 8)
        .unwrap();
    router
        .spawn_market(market_id(), AsceSwapEngine::default(), 8)
        .unwrap();
    let first = sell_order(1, market_id(), 1, 100, 40);
    let second = sell_order(2, other_market_id(), 2, 200, 80);
    let first_hash = order_hash(&first);
    let second_hash = order_hash(&second);
    router.submit_order(submit(second, 100)).await.unwrap();
    router.submit_order(submit(first, 100)).await.unwrap();

    let snapshot = router.snapshot_all().await.unwrap();

    assert_eq!(snapshot.orders.len(), 2);
    assert_eq!(
        snapshot
            .orders
            .iter()
            .map(|order| order.hash)
            .collect::<Vec<_>>(),
        sorted_hashes([first_hash, second_hash])
    );
}

#[tokio::test]
async fn router_global_snapshot_includes_active_reservations() {
    let mut router = MarketActorRouter::new();
    router
        .spawn_market(market_id(), AsceSwapEngine::default(), 8)
        .unwrap();
    let maker = sell_order(1, market_id(), 1, 100, 40);
    router.submit_order(submit(maker, 100)).await.unwrap();
    let taker = buy_order(2, market_id(), 2, 100, 50);
    let result = router.submit_order(submit(taker, 101)).await.unwrap();
    let reservation_id = match result.outcome {
        SubmitOrderOutcome::Matched { reservation_id, .. } => reservation_id,
        other => panic!("expected matched outcome, got {other:?}"),
    };

    let snapshot = router.snapshot_all().await.unwrap();

    assert_eq!(snapshot.reservations.len(), 1);
    assert_eq!(snapshot.reservations[0].id, reservation_id);
    assert_eq!(snapshot.reservations[0].status, ReservationStatus::Reserved);
}

#[tokio::test]
async fn router_spawns_one_actor_per_snapshot_market() {
    let mut engine = AsceSwapEngine::default();
    let first = sell_order(1, market_id(), 1, 100, 40);
    let second = sell_order(2, other_market_id(), 2, 200, 80);
    let first_hash = order_hash(&first);
    let second_hash = order_hash(&second);
    engine.submit_order(submit(first, 100)).unwrap();
    engine.submit_order(submit(second, 100)).unwrap();

    let mut router = MarketActorRouter::new();
    router
        .spawn_from_snapshot(engine.snapshot(), MatchConfig::default(), 8)
        .unwrap();

    assert_eq!(router.market_count(), 2);
    assert_eq!(
        router
            .order_record(first_hash)
            .await
            .unwrap()
            .unwrap()
            .state(),
        OrderState::Open
    );
    assert_eq!(
        router
            .order_record(second_hash)
            .await
            .unwrap()
            .unwrap()
            .state(),
        OrderState::Open
    );
}

fn sorted_hashes<const N: usize>(hashes: [B256; N]) -> Vec<B256> {
    let mut hashes = Vec::from(hashes);
    hashes.sort_by(|left, right| left.as_slice().cmp(right.as_slice()));
    hashes
}

#[tokio::test]
async fn router_indexes_reservations_from_snapshot() {
    let mut engine = AsceSwapEngine::default();
    let maker = sell_order(1, market_id(), 1, 100, 40);
    engine.submit_order(submit(maker, 100)).unwrap();
    let taker = buy_order(2, market_id(), 2, 100, 50);
    let result = engine.submit_order(submit(taker, 101)).unwrap();
    let reservation_id = match result.outcome {
        SubmitOrderOutcome::Matched { reservation_id, .. } => reservation_id,
        other => panic!("expected matched outcome, got {other:?}"),
    };

    let mut router = MarketActorRouter::new();
    router
        .spawn_from_snapshot(engine.snapshot(), MatchConfig::default(), 8)
        .unwrap();
    router.release_reservation(reservation_id).await.unwrap();

    assert_eq!(
        router
            .reservation(reservation_id)
            .await
            .unwrap()
            .unwrap()
            .status,
        ReservationStatus::Released
    );
}

#[test]
fn router_rejects_snapshot_reservation_with_missing_order() {
    let reservation_id = B256::repeat_byte(50);
    let missing_order = B256::repeat_byte(51);
    let snapshot = EngineSnapshot {
        orders: Vec::new(),
        reservations: vec![Reservation {
            id: reservation_id,
            status: ReservationStatus::Reserved,
            created_at: 1,
            expires_at: None,
            legs: vec![ReservationLeg {
                order_hash: missing_order,
                role: ReservationLegRole::Maker,
                claim_amount: U256::from(1),
            }],
        }],
        next_reservation_sequence: 0,
    };

    let mut router = MarketActorRouter::new();
    let error = router
        .spawn_from_snapshot(snapshot, MatchConfig::default(), 8)
        .unwrap_err();

    assert_eq!(
        error,
        MarketActorError::ReservationOrderMissing {
            reservation_id,
            order_hash: missing_order,
        }
    );
}

#[test]
fn router_rejects_snapshot_reservation_spanning_markets() {
    let first = sell_order(1, market_id(), 1, 100, 40);
    let second = sell_order(2, other_market_id(), 2, 100, 40);
    let first_hash = order_hash(&first);
    let second_hash = order_hash(&second);
    let reservation_id = B256::repeat_byte(52);
    let snapshot = EngineSnapshot {
        orders: vec![
            OrderSnapshot {
                hash: first_hash,
                order: first,
                state: OrderState::Reserved,
                filled_claim_amount: U256::ZERO,
                resting: true,
                accepted_sequence: Some(0),
            },
            OrderSnapshot {
                hash: second_hash,
                order: second,
                state: OrderState::Reserved,
                filled_claim_amount: U256::ZERO,
                resting: true,
                accepted_sequence: Some(1),
            },
        ],
        reservations: vec![Reservation {
            id: reservation_id,
            status: ReservationStatus::Reserved,
            created_at: 1,
            expires_at: None,
            legs: vec![
                ReservationLeg {
                    order_hash: first_hash,
                    role: ReservationLegRole::Maker,
                    claim_amount: U256::from(1),
                },
                ReservationLeg {
                    order_hash: second_hash,
                    role: ReservationLegRole::Maker,
                    claim_amount: U256::from(1),
                },
            ],
        }],
        next_reservation_sequence: 0,
    };

    let mut router = MarketActorRouter::new();
    let error = router
        .spawn_from_snapshot(snapshot, MatchConfig::default(), 8)
        .unwrap_err();

    assert_eq!(
        error,
        MarketActorError::ReservationSpansMarkets {
            reservation_id,
            first: market_id(),
            second: other_market_id(),
        }
    );
}
