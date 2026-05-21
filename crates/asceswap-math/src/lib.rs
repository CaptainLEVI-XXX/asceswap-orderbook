mod error;
mod fee;
mod fill_accounting;
mod fixed_point;
mod price;

pub use error::MathError;
pub use fee::taker_fee;
pub use fill_accounting::{
    collateral_delta, collateral_delta_for_new_fill, cumulative_collateral,
    new_filled_claim_amount, prepare_fill, remaining_claim_amount, PreparedFill,
};
pub use fixed_point::{mul_div_floor, BPS_DENOMINATOR, WAD};
pub use price::{price_wad, price_wad_from_amounts, Price};

#[cfg(test)]
mod tests;
