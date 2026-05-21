use asceswap_math::{price_wad, WAD};
use asceswap_orderbook::MarketOrderBook;
use asceswap_types::{MatchKind, Order, Side, U256};

use crate::plan::{MatchPlan, PlanBuilder};
use crate::validate::validate_inputs;
use crate::{MatchConfig, MatchError};

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
        if builder.maker_fill_count() == config.max_maker_orders {
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
        if builder.maker_fill_count() == config.max_maker_orders {
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
