use asceswap_math::price_wad;
use asceswap_orderbook::MarketOrderBook;
use asceswap_types::{MatchKind, Order, Side, U256};

use crate::plan::{MatchPlan, PlanBuilder};
use crate::validate::validate_inputs;
use crate::{MatchConfig, MatchError};

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
        if builder.maker_fill_count() == config.max_maker_orders {
            break;
        }

        let crossed = match taker_order.side {
            Side::Buy => taker_price >= maker.price,
            Side::Sell => taker_price <= maker.price,
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
