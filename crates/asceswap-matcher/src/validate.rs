use asceswap_math::remaining_claim_amount;
use asceswap_orderbook::MarketOrderBook;
use asceswap_types::{Order, U256};

use crate::{MatchConfig, MatchError, CONTRACT_MAX_MAKER_ORDERS};

pub(crate) fn validate_inputs(
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
