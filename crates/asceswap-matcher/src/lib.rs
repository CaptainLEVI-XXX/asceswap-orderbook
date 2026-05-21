mod assisted;
mod config;
mod direct;
mod error;
mod plan;
mod validate;

pub use assisted::{plan_merge_assisted, plan_mint_assisted};
pub use config::{MatchConfig, CONTRACT_MAX_MAKER_ORDERS};
pub use direct::plan_direct;
pub use error::MatchError;
pub use plan::{MakerFill, MatchPlan};

use asceswap_orderbook::MarketOrderBook;
use asceswap_types::{Order, Side, U256};

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

#[cfg(test)]
mod tests;
