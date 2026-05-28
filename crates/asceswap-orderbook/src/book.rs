use std::collections::{vec_deque, HashMap, VecDeque};

use asceswap_math::{prepare_fill, price_wad, remaining_claim_amount, Price};
use asceswap_types::{ClaimSide, MarketId, Order, OrderHash, Side, U256};

use crate::level::{LevelHashesIter, PriceLevelBook};
use crate::{BookError, DepthLevel, RestingOrder};

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

pub struct PriorityIter<'a> {
    orders: &'a HashMap<OrderHash, RestingOrder>,
    levels: LevelHashesIter<'a>,
    current_level: Option<vec_deque::Iter<'a, OrderHash>>,
}

impl<'a> Iterator for PriorityIter<'a> {
    type Item = &'a RestingOrder;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(current_level) = self.current_level.as_mut() {
                for hash in current_level.by_ref() {
                    if let Some(order) = self.orders.get(hash) {
                        return Some(order);
                    }
                }
            }

            self.current_level = self.levels.next().map(VecDeque::iter);
            self.current_level.as_ref()?;
        }
    }
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
        self.insert_with_sequence(hash, order, U256::ZERO, self.next_sequence)
    }

    pub fn restore(
        &mut self,
        hash: OrderHash,
        order: Order,
        filled_claim_amount: U256,
        accepted_sequence: u64,
    ) -> Result<Price, BookError> {
        self.insert_with_sequence(hash, order, filled_claim_amount, accepted_sequence)
    }

    fn insert_with_sequence(
        &mut self,
        hash: OrderHash,
        order: Order,
        filled_claim_amount: U256,
        accepted_sequence: u64,
    ) -> Result<Price, BookError> {
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
        if remaining_claim_amount(&order, filled_claim_amount)? == U256::ZERO {
            return Err(BookError::FilledOrder(hash));
        }
        let resting_order = RestingOrder {
            hash,
            order,
            filled_claim_amount,
            accepted_sequence,
            price,
        };

        let next_sequence = accepted_sequence
            .checked_add(1)
            .ok_or(BookError::SequenceOverflow)?;
        if self.next_sequence < next_sequence {
            self.next_sequence = next_sequence;
        }

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
        self.iter_priority(claim, side).next()
    }

    pub fn iter_priority(&self, claim: ClaimSide, side: Side) -> PriorityIter<'_> {
        let side_book = self.side(claim, side);
        PriorityIter {
            orders: &self.orders,
            levels: side_book.hashes_by_priority(side),
            current_level: None,
        }
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
