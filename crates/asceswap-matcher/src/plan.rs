use asceswap_math::{collateral_delta, new_filled_claim_amount, remaining_claim_amount, MathError};
use asceswap_orderbook::RestingOrder;
use asceswap_types::{MatchKind, Order, OrderHash, Side, U256};

use crate::MatchError;

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

#[derive(Clone, Debug)]
pub(crate) struct PlanBuilder {
    match_kind: MatchKind,
    taker_claim_fill_amount: U256,
    total_maker_collateral_amount: U256,
    maker_fills: Vec<MakerFill>,
}

impl PlanBuilder {
    pub(crate) fn new(match_kind: MatchKind) -> Self {
        Self {
            match_kind,
            taker_claim_fill_amount: U256::ZERO,
            total_maker_collateral_amount: U256::ZERO,
            maker_fills: Vec::new(),
        }
    }

    pub(crate) fn maker_fill_count(&self) -> usize {
        self.maker_fills.len()
    }

    pub(crate) fn push_fill(
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

    pub(crate) fn finish(
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
