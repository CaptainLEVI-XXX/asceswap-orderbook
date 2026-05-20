use asceswap_math::{
    collateral_delta, new_filled_claim_amount, price_wad, remaining_claim_amount, MathError, WAD,
};
use asceswap_orderbook::{MarketOrderBook, RestingOrder};
use asceswap_types::{MarketId, MatchKind, Order, OrderError, OrderHash, Side, U256};

pub const CONTRACT_MAX_MAKER_ORDERS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchConfig {
    pub max_maker_orders: usize,
}

impl Default for MatchConfig {
    fn default() -> Self {
        Self {
            max_maker_orders: CONTRACT_MAX_MAKER_ORDERS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchError {
    InvalidConfig,
    WrongMarket { expected: MarketId, got: MarketId },
    InvalidTaker(OrderError),
    Math(MathError),
}

impl From<MathError> for MatchError {
    fn from(error: MathError) -> Self {
        Self::Math(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MakerFill {
    pub order_hash: OrderHash,
    pub claim_fill_amount: U256,
    pub collateral_amount: U256,
    pub new_filled_claim_amount: U256,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchPlan {
    pub match_kind: MatchKind,
    pub taker_claim_fill_amount: U256,
    pub taker_collateral_amount: U256,
    pub taker_actual_collateral_amount: U256,
    pub total_maker_claim_fill_amount: U256,
    pub total_maker_collateral_amount: U256,
    pub maker_fills: Vec<MakerFill>,
}

pub fn plan_match(
    book: &MarketOrderBook,
    taker_order: &Order,
    taker_filled_claim_amount: U256,
    config: MatchConfig,
) -> Result<Option<MatchPlan>, MatchError> {
    if let Some(plan) = plan_direct(book, taker_order, taker_filled_claim_amount, config)? {
        return Ok(Some(plan));
    }

    match taker_order.side {
        Side::Buy => plan_mint_assisted(book, taker_order, taker_filled_claim_amount, config),
        Side::Sell => plan_merge_assisted(book, taker_order, taker_filled_claim_amount, config),
    }
}

pub fn plan_direct(
    book: &MarketOrderBook,
    taker_order: &Order,
    taker_filled_claim_amount: U256,
    config: MatchConfig,
) -> Result<Option<MatchPlan>, MatchError> {
    validate_inputs(book, taker_order, taker_filled_claim_amount, config)?;

    let taker_price = price_wad(taker_order)?;
    let mut builder = PlanBuilder::new(MatchKind::Direct);

    for maker in book.iter_priority(taker_order.claim, taker_order.side.opposite()) {
        if builder.maker_fills.len() == config.max_maker_orders {
            break;
        }

        let maker_price = maker.price;
        let crossed = match taker_order.side {
            Side::Buy => taker_price >= maker_price,
            Side::Sell => taker_price <= maker_price,
        };
        if !crossed {
            break;
        }

        if !builder.push_fill(maker, taker_order, taker_filled_claim_amount)? {
            break;
        }
    }

    builder.finish(taker_order, taker_filled_claim_amount)
}

pub fn plan_mint_assisted(
    book: &MarketOrderBook,
    taker_order: &Order,
    taker_filled_claim_amount: U256,
    config: MatchConfig,
) -> Result<Option<MatchPlan>, MatchError> {
    validate_inputs(book, taker_order, taker_filled_claim_amount, config)?;
    if taker_order.side != Side::Buy {
        return Ok(None);
    }

    let taker_price = price_wad(taker_order)?;
    let mut builder = PlanBuilder::new(MatchKind::MintAssisted);

    for maker in book.iter_priority(taker_order.claim.opposite(), Side::Buy) {
        if builder.maker_fills.len() == config.max_maker_orders {
            break;
        }

        if maker.price.wad() < WAD - taker_price.wad() {
            break;
        }

        if !builder.push_fill(maker, taker_order, taker_filled_claim_amount)? {
            break;
        }
    }

    builder.finish(taker_order, taker_filled_claim_amount)
}

pub fn plan_merge_assisted(
    book: &MarketOrderBook,
    taker_order: &Order,
    taker_filled_claim_amount: U256,
    config: MatchConfig,
) -> Result<Option<MatchPlan>, MatchError> {
    validate_inputs(book, taker_order, taker_filled_claim_amount, config)?;
    if taker_order.side != Side::Sell {
        return Ok(None);
    }

    let taker_price = price_wad(taker_order)?;
    let mut builder = PlanBuilder::new(MatchKind::MergeAssisted);

    for maker in book.iter_priority(taker_order.claim.opposite(), Side::Sell) {
        if builder.maker_fills.len() == config.max_maker_orders {
            break;
        }

        if maker.price.wad() > WAD - taker_price.wad() {
            break;
        }

        if !builder.push_fill(maker, taker_order, taker_filled_claim_amount)? {
            break;
        }
    }

    builder.finish(taker_order, taker_filled_claim_amount)
}

fn validate_inputs(
    book: &MarketOrderBook,
    taker_order: &Order,
    taker_filled_claim_amount: U256,
    config: MatchConfig,
) -> Result<(), MatchError> {
    if config.max_maker_orders == 0 || config.max_maker_orders > CONTRACT_MAX_MAKER_ORDERS {
        return Err(MatchError::InvalidConfig);
    }
    taker_order
        .validate_basic()
        .map_err(MatchError::InvalidTaker)?;
    if taker_order.market_id != book.market_id() {
        return Err(MatchError::WrongMarket {
            expected: book.market_id(),
            got: taker_order.market_id,
        });
    }
    remaining_claim_amount(taker_order, taker_filled_claim_amount)?;

    Ok(())
}

#[derive(Clone, Debug)]
struct PlanBuilder {
    match_kind: MatchKind,
    taker_claim_fill_amount: U256,
    total_maker_collateral_amount: U256,
    maker_fills: Vec<MakerFill>,
}

impl PlanBuilder {
    fn new(match_kind: MatchKind) -> Self {
        Self {
            match_kind,
            taker_claim_fill_amount: U256::ZERO,
            total_maker_collateral_amount: U256::ZERO,
            maker_fills: Vec::new(),
        }
    }

    fn push_fill(
        &mut self,
        maker: &RestingOrder,
        taker_order: &Order,
        taker_filled_claim_amount: U256,
    ) -> Result<bool, MatchError> {
        let taker_available = remaining_claim_amount(taker_order, taker_filled_claim_amount)?
            - self.taker_claim_fill_amount;
        if taker_available == U256::ZERO {
            return Ok(false);
        }

        let maker_available = maker.remaining_claim_amount()?;
        let claim_fill_amount = taker_available.min(maker_available);
        if claim_fill_amount == U256::ZERO {
            return Ok(true);
        }

        let maker_new_filled_claim_amount =
            new_filled_claim_amount(&maker.order, maker.filled_claim_amount, claim_fill_amount)?;
        let maker_collateral_amount =
            collateral_delta(&maker.order, maker.filled_claim_amount, claim_fill_amount)?;

        self.taker_claim_fill_amount = self
            .taker_claim_fill_amount
            .checked_add(claim_fill_amount)
            .ok_or(MathError::ArithmeticOverflow)?;
        self.total_maker_collateral_amount = self
            .total_maker_collateral_amount
            .checked_add(maker_collateral_amount)
            .ok_or(MathError::ArithmeticOverflow)?;
        self.maker_fills.push(MakerFill {
            order_hash: maker.hash,
            claim_fill_amount,
            collateral_amount: maker_collateral_amount,
            new_filled_claim_amount: maker_new_filled_claim_amount,
        });

        Ok(true)
    }

    fn finish(
        self,
        taker_order: &Order,
        taker_filled_claim_amount: U256,
    ) -> Result<Option<MatchPlan>, MatchError> {
        if self.taker_claim_fill_amount == U256::ZERO {
            return Ok(None);
        }

        let taker_collateral_amount = collateral_delta(
            taker_order,
            taker_filled_claim_amount,
            self.taker_claim_fill_amount,
        )?;

        let taker_actual_collateral_amount = match self.match_kind {
            MatchKind::Direct => {
                let valid = match taker_order.side {
                    Side::Buy => self.total_maker_collateral_amount <= taker_collateral_amount,
                    Side::Sell => self.total_maker_collateral_amount >= taker_collateral_amount,
                };
                if !valid {
                    return Ok(None);
                }
                self.total_maker_collateral_amount
            }
            MatchKind::MintAssisted => {
                if self.total_maker_collateral_amount > self.taker_claim_fill_amount {
                    return Ok(None);
                }
                let taker_actual_cost =
                    self.taker_claim_fill_amount - self.total_maker_collateral_amount;
                if taker_actual_cost > taker_collateral_amount {
                    return Ok(None);
                }
                taker_actual_cost
            }
            MatchKind::MergeAssisted => {
                if self.total_maker_collateral_amount > self.taker_claim_fill_amount {
                    return Ok(None);
                }
                let taker_gross_proceeds =
                    self.taker_claim_fill_amount - self.total_maker_collateral_amount;
                if taker_gross_proceeds < taker_collateral_amount {
                    return Ok(None);
                }
                taker_gross_proceeds
            }
        };

        Ok(Some(MatchPlan {
            match_kind: self.match_kind,
            taker_claim_fill_amount: self.taker_claim_fill_amount,
            taker_collateral_amount,
            taker_actual_collateral_amount,
            total_maker_claim_fill_amount: self.taker_claim_fill_amount,
            total_maker_collateral_amount: self.total_maker_collateral_amount,
            maker_fills: self.maker_fills,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asceswap_orderbook::MarketOrderBook;
    use asceswap_types::{Address, ClaimSide, B256};

    fn market() -> MarketId {
        B256::repeat_byte(9)
    }

    fn hash(value: u8) -> OrderHash {
        B256::repeat_byte(value)
    }

    fn order(
        claim: ClaimSide,
        side: Side,
        maker_amount: u64,
        taker_amount: u64,
        maker: u8,
    ) -> Order {
        Order {
            salt: U256::from(maker),
            maker: Address::repeat_byte(maker),
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
    fn plans_direct_buy_with_price_improvement() {
        let mut book = MarketOrderBook::new(market());
        book.insert(hash(1), order(ClaimSide::Payoff, Side::Sell, 100, 49, 1))
            .unwrap();
        let taker = order(ClaimSide::Payoff, Side::Buy, 50, 100, 2);

        let plan = plan_direct(&book, &taker, U256::ZERO, MatchConfig::default())
            .unwrap()
            .unwrap();

        assert_eq!(plan.match_kind, MatchKind::Direct);
        assert_eq!(plan.taker_claim_fill_amount, U256::from(100));
        assert_eq!(plan.taker_collateral_amount, U256::from(50));
        assert_eq!(plan.total_maker_collateral_amount, U256::from(49));
        assert_eq!(plan.taker_actual_collateral_amount, U256::from(49));
        assert_eq!(plan.maker_fills[0].order_hash, hash(1));
    }

    #[test]
    fn plans_direct_sell_with_price_improvement() {
        let mut book = MarketOrderBook::new(market());
        book.insert(hash(1), order(ClaimSide::Payoff, Side::Buy, 50, 100, 1))
            .unwrap();
        let taker = order(ClaimSide::Payoff, Side::Sell, 100, 48, 2);

        let plan = plan_direct(&book, &taker, U256::ZERO, MatchConfig::default())
            .unwrap()
            .unwrap();

        assert_eq!(plan.match_kind, MatchKind::Direct);
        assert_eq!(plan.taker_claim_fill_amount, U256::from(100));
        assert_eq!(plan.taker_collateral_amount, U256::from(48));
        assert_eq!(plan.total_maker_collateral_amount, U256::from(50));
        assert_eq!(plan.taker_actual_collateral_amount, U256::from(50));
    }

    #[test]
    fn plans_mint_assisted_buy_buy() {
        let mut book = MarketOrderBook::new(market());
        book.insert(hash(1), order(ClaimSide::Residual, Side::Buy, 45, 100, 1))
            .unwrap();
        let taker = order(ClaimSide::Payoff, Side::Buy, 60, 100, 2);

        let plan = plan_mint_assisted(&book, &taker, U256::ZERO, MatchConfig::default())
            .unwrap()
            .unwrap();

        assert_eq!(plan.match_kind, MatchKind::MintAssisted);
        assert_eq!(plan.taker_claim_fill_amount, U256::from(100));
        assert_eq!(plan.taker_collateral_amount, U256::from(60));
        assert_eq!(plan.total_maker_collateral_amount, U256::from(45));
        assert_eq!(plan.taker_actual_collateral_amount, U256::from(55));
    }

    #[test]
    fn plans_merge_assisted_sell_sell() {
        let mut book = MarketOrderBook::new(market());
        book.insert(hash(1), order(ClaimSide::Residual, Side::Sell, 100, 35, 1))
            .unwrap();
        let taker = order(ClaimSide::Payoff, Side::Sell, 100, 60, 2);

        let plan = plan_merge_assisted(&book, &taker, U256::ZERO, MatchConfig::default())
            .unwrap()
            .unwrap();

        assert_eq!(plan.match_kind, MatchKind::MergeAssisted);
        assert_eq!(plan.taker_claim_fill_amount, U256::from(100));
        assert_eq!(plan.taker_collateral_amount, U256::from(60));
        assert_eq!(plan.total_maker_collateral_amount, U256::from(35));
        assert_eq!(plan.taker_actual_collateral_amount, U256::from(65));
    }

    #[test]
    fn returns_none_when_prices_do_not_cross() {
        let mut book = MarketOrderBook::new(market());
        book.insert(hash(1), order(ClaimSide::Payoff, Side::Sell, 100, 51, 1))
            .unwrap();
        let taker = order(ClaimSide::Payoff, Side::Buy, 50, 100, 2);

        assert_eq!(
            plan_direct(&book, &taker, U256::ZERO, MatchConfig::default()).unwrap(),
            None
        );
    }

    #[test]
    fn respects_contract_maker_limit() {
        let mut book = MarketOrderBook::new(market());
        for index in 1..=40_u8 {
            book.insert(
                hash(index),
                order(ClaimSide::Payoff, Side::Sell, 1, 1, index),
            )
            .unwrap();
        }
        let taker = order(ClaimSide::Payoff, Side::Buy, 40, 40, 200);

        let plan = plan_direct(&book, &taker, U256::ZERO, MatchConfig::default())
            .unwrap()
            .unwrap();

        assert_eq!(plan.maker_fills.len(), CONTRACT_MAX_MAKER_ORDERS);
        assert_eq!(plan.taker_claim_fill_amount, U256::from(32));
    }

    #[test]
    fn rejects_invalid_config_above_contract_limit() {
        let book = MarketOrderBook::new(market());
        let taker = order(ClaimSide::Payoff, Side::Buy, 50, 100, 2);

        assert_eq!(
            plan_match(
                &book,
                &taker,
                U256::ZERO,
                MatchConfig {
                    max_maker_orders: CONTRACT_MAX_MAKER_ORDERS + 1
                },
            ),
            Err(MatchError::InvalidConfig)
        );
    }
}
