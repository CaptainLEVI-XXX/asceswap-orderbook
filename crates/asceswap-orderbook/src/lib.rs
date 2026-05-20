use std::collections::{BTreeMap, HashMap, VecDeque};

use asceswap_math::{prepare_fill, price_wad, remaining_claim_amount, MathError, Price};
use asceswap_types::{ClaimSide, MarketId, Order, OrderError, OrderHash, Side, U256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookError {
    WrongMarket { expected: MarketId, got: MarketId },
    DuplicateOrder(OrderHash),
    MissingOrder(OrderHash),
    FilledOrder(OrderHash),
    InvalidOrder(OrderError),
    Math(MathError),
    SequenceOverflow,
    ArithmeticOverflow,
}

impl From<OrderError> for BookError {
    fn from(error: OrderError) -> Self {
        Self::InvalidOrder(error)
    }
}

impl From<MathError> for BookError {
    fn from(error: MathError) -> Self {
        Self::Math(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestingOrder {
    pub hash: OrderHash,
    pub order: Order,
    pub filled_claim_amount: U256,
    pub accepted_sequence: u64,
    pub price: Price,
}

impl RestingOrder {
    pub fn remaining_claim_amount(&self) -> Result<U256, MathError> {
        remaining_claim_amount(&self.order, self.filled_claim_amount)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DepthLevel {
    pub price: Price,
    pub total_claim_amount: U256,
    pub order_count: usize,
}

#[derive(Clone, Debug, Default)]
struct PriceLevelBook {
    levels: BTreeMap<Price, VecDeque<OrderHash>>,
}

impl PriceLevelBook {
    fn insert(&mut self, price: Price, hash: OrderHash) {
        self.levels.entry(price).or_default().push_back(hash);
    }

    fn remove(&mut self, price: Price, hash: OrderHash) {
        let mut remove_level = false;
        if let Some(level) = self.levels.get_mut(&price) {
            if let Some(index) = level.iter().position(|candidate| *candidate == hash) {
                level.remove(index);
            }
            remove_level = level.is_empty();
        }

        if remove_level {
            self.levels.remove(&price);
        }
    }
}

#[derive(Clone, Debug)]
pub struct MarketOrderBook {
    market_id: MarketId,
    next_sequence: u64,
    payoff_bids: PriceLevelBook,
    payoff_asks: PriceLevelBook,
    residual_bids: PriceLevelBook,
    residual_asks: PriceLevelBook,
    orders: HashMap<OrderHash, RestingOrder>,
}

impl MarketOrderBook {
    pub fn new(market_id: MarketId) -> Self {
        Self {
            market_id,
            next_sequence: 0,
            payoff_bids: PriceLevelBook::default(),
            payoff_asks: PriceLevelBook::default(),
            residual_bids: PriceLevelBook::default(),
            residual_asks: PriceLevelBook::default(),
            orders: HashMap::new(),
        }
    }

    pub fn market_id(&self) -> MarketId {
        self.market_id
    }

    pub fn order_count(&self) -> usize {
        self.orders.len()
    }

    pub fn insert(&mut self, hash: OrderHash, order: Order) -> Result<Price, BookError> {
        order.validate_basic()?;
        if order.market_id != self.market_id {
            return Err(BookError::WrongMarket {
                expected: self.market_id,
                got: order.market_id,
            });
        }
        if self.orders.contains_key(&hash) {
            return Err(BookError::DuplicateOrder(hash));
        }

        let price = price_wad(&order)?;
        let resting_order = RestingOrder {
            hash,
            order,
            filled_claim_amount: U256::ZERO,
            accepted_sequence: self.next_sequence,
            price,
        };

        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(BookError::SequenceOverflow)?;

        self.side_mut(resting_order.order.claim, resting_order.order.side)
            .insert(price, hash);
        self.orders.insert(hash, resting_order);

        Ok(price)
    }

    pub fn remove(&mut self, hash: OrderHash) -> Result<RestingOrder, BookError> {
        let resting_order = self
            .orders
            .remove(&hash)
            .ok_or(BookError::MissingOrder(hash))?;

        self.side_mut(resting_order.order.claim, resting_order.order.side)
            .remove(resting_order.price, hash);

        Ok(resting_order)
    }

    pub fn apply_fill(
        &mut self,
        hash: OrderHash,
        claim_fill_amount: U256,
    ) -> Result<U256, BookError> {
        let (new_filled_claim_amount, fully_filled) = {
            let resting_order = self
                .orders
                .get_mut(&hash)
                .ok_or(BookError::MissingOrder(hash))?;
            let fill = prepare_fill(
                &resting_order.order,
                resting_order.filled_claim_amount,
                claim_fill_amount,
            )?;
            resting_order.filled_claim_amount = fill.new_filled_claim_amount;

            (
                fill.new_filled_claim_amount,
                fill.new_filled_claim_amount == resting_order.order.max_claim_amount(),
            )
        };

        if fully_filled {
            self.remove(hash)?;
        }

        Ok(new_filled_claim_amount)
    }

    pub fn get(&self, hash: OrderHash) -> Option<&RestingOrder> {
        self.orders.get(&hash)
    }

    pub fn contains(&self, hash: OrderHash) -> bool {
        self.orders.contains_key(&hash)
    }

    pub fn best(&self, claim: ClaimSide, side: Side) -> Option<&RestingOrder> {
        self.iter_priority(claim, side).into_iter().next()
    }

    pub fn iter_priority(&self, claim: ClaimSide, side: Side) -> Vec<&RestingOrder> {
        let side_book = self.side(claim, side);
        let mut orders = Vec::new();

        match side {
            Side::Buy => {
                for (_price, hashes) in side_book.levels.iter().rev() {
                    for hash in hashes {
                        if let Some(order) = self.orders.get(hash) {
                            orders.push(order);
                        }
                    }
                }
            }
            Side::Sell => {
                for (_price, hashes) in &side_book.levels {
                    for hash in hashes {
                        if let Some(order) = self.orders.get(hash) {
                            orders.push(order);
                        }
                    }
                }
            }
        }

        orders
    }

    pub fn depth(&self, claim: ClaimSide, side: Side) -> Result<Vec<DepthLevel>, BookError> {
        let side_book = self.side(claim, side);
        let mut depth = Vec::new();

        match side {
            Side::Buy => {
                for (price, hashes) in side_book.levels.iter().rev() {
                    depth.push(self.depth_level(*price, hashes)?);
                }
            }
            Side::Sell => {
                for (price, hashes) in &side_book.levels {
                    depth.push(self.depth_level(*price, hashes)?);
                }
            }
        }

        Ok(depth)
    }

    fn depth_level(
        &self,
        price: Price,
        hashes: &VecDeque<OrderHash>,
    ) -> Result<DepthLevel, BookError> {
        let mut total_claim_amount = U256::ZERO;
        let mut order_count = 0;

        for hash in hashes {
            let resting_order = self
                .orders
                .get(hash)
                .ok_or(BookError::MissingOrder(*hash))?;
            let remaining = resting_order.remaining_claim_amount()?;
            total_claim_amount = total_claim_amount
                .checked_add(remaining)
                .ok_or(BookError::ArithmeticOverflow)?;
            order_count += 1;
        }

        Ok(DepthLevel {
            price,
            total_claim_amount,
            order_count,
        })
    }

    fn side(&self, claim: ClaimSide, side: Side) -> &PriceLevelBook {
        match (claim, side) {
            (ClaimSide::Payoff, Side::Buy) => &self.payoff_bids,
            (ClaimSide::Payoff, Side::Sell) => &self.payoff_asks,
            (ClaimSide::Residual, Side::Buy) => &self.residual_bids,
            (ClaimSide::Residual, Side::Sell) => &self.residual_asks,
        }
    }

    fn side_mut(&mut self, claim: ClaimSide, side: Side) -> &mut PriceLevelBook {
        match (claim, side) {
            (ClaimSide::Payoff, Side::Buy) => &mut self.payoff_bids,
            (ClaimSide::Payoff, Side::Sell) => &mut self.payoff_asks,
            (ClaimSide::Residual, Side::Buy) => &mut self.residual_bids,
            (ClaimSide::Residual, Side::Sell) => &mut self.residual_asks,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asceswap_types::{Address, B256};

    fn market() -> MarketId {
        B256::repeat_byte(9)
    }

    fn hash(value: u8) -> OrderHash {
        B256::repeat_byte(value)
    }

    fn order(claim: ClaimSide, side: Side, maker_amount: u64, taker_amount: u64) -> Order {
        Order {
            salt: U256::from(1),
            maker: Address::repeat_byte(1),
            market_id: market(),
            claim,
            maker_amount: U256::from(maker_amount),
            taker_amount: U256::from(taker_amount),
            side,
            expiration: U256::ZERO,
            epoch: U256::ZERO,
            max_fee_rate_bps: 100,
        }
    }

    #[test]
    fn keeps_price_time_priority_for_bids_and_asks() {
        let mut book = MarketOrderBook::new(market());
        book.insert(hash(1), order(ClaimSide::Payoff, Side::Sell, 100, 50))
            .unwrap();
        book.insert(hash(2), order(ClaimSide::Payoff, Side::Sell, 100, 49))
            .unwrap();
        book.insert(hash(3), order(ClaimSide::Payoff, Side::Sell, 100, 49))
            .unwrap();
        book.insert(hash(4), order(ClaimSide::Payoff, Side::Buy, 51, 100))
            .unwrap();
        book.insert(hash(5), order(ClaimSide::Payoff, Side::Buy, 50, 100))
            .unwrap();

        let asks: Vec<_> = book
            .iter_priority(ClaimSide::Payoff, Side::Sell)
            .into_iter()
            .map(|order| order.hash)
            .collect();
        assert_eq!(asks, vec![hash(2), hash(3), hash(1)]);

        let bids: Vec<_> = book
            .iter_priority(ClaimSide::Payoff, Side::Buy)
            .into_iter()
            .map(|order| order.hash)
            .collect();
        assert_eq!(bids, vec![hash(4), hash(5)]);
    }

    #[test]
    fn aggregates_depth_at_price() {
        let mut book = MarketOrderBook::new(market());
        book.insert(hash(1), order(ClaimSide::Payoff, Side::Sell, 100, 49))
            .unwrap();
        book.insert(hash(2), order(ClaimSide::Payoff, Side::Sell, 200, 98))
            .unwrap();

        let depth = book.depth(ClaimSide::Payoff, Side::Sell).unwrap();
        assert_eq!(depth.len(), 1);
        assert_eq!(depth[0].total_claim_amount, U256::from(300));
        assert_eq!(depth[0].order_count, 2);
    }

    #[test]
    fn removes_fully_filled_order() {
        let mut book = MarketOrderBook::new(market());
        book.insert(hash(1), order(ClaimSide::Payoff, Side::Sell, 100, 49))
            .unwrap();

        assert_eq!(
            book.apply_fill(hash(1), U256::from(100)).unwrap(),
            U256::from(100)
        );
        assert!(!book.contains(hash(1)));
        assert!(book.best(ClaimSide::Payoff, Side::Sell).is_none());
    }

    #[test]
    fn rejects_wrong_market_and_duplicates() {
        let mut book = MarketOrderBook::new(market());
        let mut wrong_market = order(ClaimSide::Payoff, Side::Sell, 100, 49);
        wrong_market.market_id = B256::repeat_byte(8);

        assert!(matches!(
            book.insert(hash(1), wrong_market),
            Err(BookError::WrongMarket { .. })
        ));

        let sell = order(ClaimSide::Payoff, Side::Sell, 100, 49);
        book.insert(hash(1), sell.clone()).unwrap();
        assert_eq!(
            book.insert(hash(1), sell),
            Err(BookError::DuplicateOrder(hash(1)))
        );
    }
}
