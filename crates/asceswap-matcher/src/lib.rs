mod assisted;
mod config;
mod direct;
mod error;
mod plan;
mod validate;

pub use assisted::{
    plan_merge_assisted, plan_merge_assisted_with_filter, plan_mint_assisted,
    plan_mint_assisted_with_filter,
};
pub use config::{MatchConfig, CONTRACT_MAX_MAKER_ORDERS};
pub use direct::{plan_direct, plan_direct_with_filter};
pub use error::MatchError;
pub use plan::{MakerFill, MatchPlan};

use asceswap_orderbook::{MarketOrderBook, RestingOrder};
use asceswap_types::{Order, Side, U256};

pub fn plan_match(
    book: &MarketOrderBook,
    taker_order: &Order,
    taker_filled_claim_amount: U256,
    config: MatchConfig,
) -> Result<Option<MatchPlan>, MatchError> {
    plan_match_with_filter(book, taker_order, taker_filled_claim_amount, config, |_| {
        true
    })
}

pub fn plan_match_with_filter<F>(
    book: &MarketOrderBook,
    taker_order: &Order,
    taker_filled_claim_amount: U256,
    config: MatchConfig,
    maker_filter: F,
) -> Result<Option<MatchPlan>, MatchError>
where
    F: Fn(&RestingOrder) -> bool,
{
    if let Some(plan) = plan_direct_with_filter(
        book,
        taker_order,
        taker_filled_claim_amount,
        config,
        &maker_filter,
    )? {
        return Ok(Some(plan));
    }

    match taker_order.side {
        Side::Buy => plan_mint_assisted_with_filter(
            book,
            taker_order,
            taker_filled_claim_amount,
            config,
            maker_filter,
        ),
        Side::Sell => plan_merge_assisted_with_filter(
            book,
            taker_order,
            taker_filled_claim_amount,
            config,
            maker_filter,
        ),
    }
}

#[cfg(test)]
mod tests;
