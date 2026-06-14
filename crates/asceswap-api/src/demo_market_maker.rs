use asceswap_engine::{EngineError, SubmitOrder};
use asceswap_math::{collateral_delta, price_wad_from_amounts, remaining_claim_amount, WAD};
use asceswap_types::{Address, ClaimSide, Order, Side, U256, U512};
use asceswap_validation::{
    order_digest, order_hash, OrderValidationContext, SignatureCheck, SignatureDomain,
};
use k256::ecdsa::SigningKey;

use crate::ApiError;

#[derive(Clone)]
pub struct DemoMarketMaker {
    private_key: [u8; 32],
    maker: Address,
    signature_domain: SignatureDomain,
    epoch: U256,
    max_fee_rate_bps: u16,
    reservation_ttl_secs: Option<u64>,
    auto_commit: bool,
    next_salt: u64,
}

impl std::fmt::Debug for DemoMarketMaker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DemoMarketMaker")
            .field("maker", &self.maker)
            .field("signature_domain", &self.signature_domain)
            .field("epoch", &self.epoch)
            .field("max_fee_rate_bps", &self.max_fee_rate_bps)
            .field("reservation_ttl_secs", &self.reservation_ttl_secs)
            .field("auto_commit", &self.auto_commit)
            .field("next_salt", &self.next_salt)
            .finish_non_exhaustive()
    }
}

impl DemoMarketMaker {
    pub fn new(
        private_key: [u8; 32],
        signature_domain: SignatureDomain,
        epoch: U256,
        max_fee_rate_bps: u16,
        reservation_ttl_secs: Option<u64>,
        auto_commit: bool,
    ) -> Result<Self, ApiError> {
        let signing_key =
            SigningKey::from_bytes((&private_key).into()).map_err(|_| ApiError::InvalidField {
                field: "demo_market_maker_private_key",
                reason: "invalid secp256k1 private key",
            })?;
        let maker = Address::from_public_key(signing_key.verifying_key());

        Ok(Self {
            private_key,
            maker,
            signature_domain,
            epoch,
            max_fee_rate_bps,
            reservation_ttl_secs,
            auto_commit,
            next_salt: 1,
        })
    }

    pub fn maker(&self) -> Address {
        self.maker
    }

    pub fn auto_commit(&self) -> bool {
        self.auto_commit
    }

    pub fn ensure_next_salt_at_least(&mut self, salt: u64) {
        self.next_salt = self.next_salt.max(salt);
    }

    pub fn counter_order_for(
        &mut self,
        taker_order: &Order,
        taker_filled_claim_amount: U256,
        now: u64,
    ) -> Result<SubmitOrder, ApiError> {
        let claim_amount = remaining_claim_amount(taker_order, taker_filled_claim_amount)
            .map_err(EngineError::from)?;
        let collateral_amount =
            collateral_delta(taker_order, taker_filled_claim_amount, claim_amount)
                .map_err(EngineError::from)?;

        let (claim, side, maker_amount, taker_amount) = demo_counter_order_terms(
            taker_order.claim,
            taker_order.side,
            claim_amount,
            collateral_amount,
        )?;

        let salt = self.next_salt;
        self.next_salt = self
            .next_salt
            .checked_add(1)
            .ok_or(ApiError::SequenceOverflow)?;

        let order = Order {
            salt: U256::from(salt),
            maker: self.maker,
            market_id: taker_order.market_id,
            claim,
            maker_amount,
            taker_amount,
            side,
            expiration: U256::ZERO,
            epoch: self.epoch,
            max_fee_rate_bps: self.max_fee_rate_bps,
        };
        let signature = self.sign_order(&order)?;
        let validation = OrderValidationContext::new(now)
            .with_expected_order_hash(order_hash(&order))
            .with_maker_epoch(order.epoch)
            .with_fee_rate_bps(0)
            .with_signature(SignatureCheck::Valid)
            .with_required_signature(true);

        Ok(SubmitOrder::new(order, validation)
            .with_signature(Some(signature))
            .with_rest_on_no_match(false)
            .with_reservation_ttl_secs(self.reservation_ttl_secs))
    }

    fn sign_order(&self, order: &Order) -> Result<Vec<u8>, ApiError> {
        let signing_key = SigningKey::from_bytes((&self.private_key).into()).map_err(|_| {
            ApiError::InvalidField {
                field: "demo_market_maker_private_key",
                reason: "invalid secp256k1 private key",
            }
        })?;
        let digest = order_digest(order, self.signature_domain);
        let (signature, recovery_id) = signing_key
            .sign_prehash_recoverable(digest.as_slice())
            .map_err(|_| ApiError::InvalidField {
                field: "demo_market_maker_private_key",
                reason: "failed to sign market-maker order",
            })?;

        let mut signature_bytes = Vec::with_capacity(65);
        signature_bytes.extend_from_slice(&signature.to_bytes());
        signature_bytes.push(27 + u8::from(recovery_id));
        Ok(signature_bytes)
    }
}

fn demo_counter_order_terms(
    claim: ClaimSide,
    side: Side,
    claim_amount: U256,
    collateral_amount: U256,
) -> Result<(ClaimSide, Side, U256, U256), ApiError> {
    match side {
        Side::Buy => {
            let taker_price = price_wad_from_amounts(collateral_amount, claim_amount)
                .map_err(|error| ApiError::Engine(EngineError::Math(error)))?;
            let required_price = WAD
                .checked_sub(taker_price.wad())
                .ok_or(ApiError::Engine(EngineError::ArithmeticOverflow))?;
            let maker_amount = mul_div_ceil(required_price, claim_amount, WAD)?;
            Ok((
                claim.opposite(),
                Side::Buy,
                maker_amount,
                claim_amount,
            ))
        }
        Side::Sell => Ok((claim, Side::Buy, collateral_amount, claim_amount)),
    }
}

fn mul_div_ceil(a: U256, b: U256, denominator: U256) -> Result<U256, ApiError> {
    if denominator == U256::ZERO {
        return Err(ApiError::Engine(EngineError::ArithmeticOverflow));
    }

    let product: U512 = a.widening_mul(b);
    let denominator = U512::from(denominator);
    let mut quotient = product / denominator;
    if product % denominator != U512::ZERO {
        quotient += U512::from(1);
    }
    let limbs = quotient.as_limbs();
    if limbs[4..].iter().any(|limb| *limb != 0) {
        return Err(ApiError::Engine(EngineError::ArithmeticOverflow));
    }

    Ok(U256::from_limbs([limbs[0], limbs[1], limbs[2], limbs[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use asceswap_matcher::{plan_match, MatchConfig};
    use asceswap_orderbook::MarketOrderBook;
    use asceswap_types::{ClaimSide, B256};

    #[test]
    fn user_buy_is_matched_with_opposite_claim_buy() {
        let (claim, side, maker_amount, taker_amount) = demo_counter_order_terms(
            ClaimSide::Payoff,
            Side::Buy,
            U256::from(100),
            U256::from(45),
        )
        .unwrap();

        assert_eq!(claim, ClaimSide::Residual);
        assert_eq!(side, Side::Buy);
        assert_eq!(maker_amount, U256::from(55));
        assert_eq!(taker_amount, U256::from(100));
    }

    #[test]
    fn user_buy_counter_order_uses_minimum_collateral_to_cross_after_rounding() {
        let (claim, side, maker_amount, taker_amount) = demo_counter_order_terms(
            ClaimSide::Payoff,
            Side::Buy,
            U256::from(2_222_222),
            U256::from(1_000_000),
        )
        .unwrap();

        assert_eq!(claim, ClaimSide::Residual);
        assert_eq!(side, Side::Buy);
        assert_eq!(maker_amount, U256::from(1_222_223));
        assert_eq!(taker_amount, U256::from(2_222_222));
    }

    #[test]
    fn user_buy_counter_order_crosses_for_varied_amounts() {
        let market_id = B256::repeat_byte(7);
        let maker = Address::repeat_byte(1);
        let mm = Address::repeat_byte(2);
        let claim_amounts = [3_u64, 7, 10, 101, 1_000, 2_222_222, 9_999_999];

        for claim_amount in claim_amounts {
            let collateral_amounts = [
                1,
                claim_amount / 4,
                claim_amount / 2,
                claim_amount.saturating_sub(1),
            ];
            for collateral_amount in collateral_amounts {
                if collateral_amount == 0 || collateral_amount >= claim_amount {
                    continue;
                }
                let user_order = Order {
                    salt: U256::from(collateral_amount),
                    maker,
                    market_id,
                    claim: ClaimSide::Payoff,
                    maker_amount: U256::from(collateral_amount),
                    taker_amount: U256::from(claim_amount),
                    side: Side::Buy,
                    expiration: U256::ZERO,
                    epoch: U256::ZERO,
                    max_fee_rate_bps: 100,
                };
                let (claim, side, maker_amount, taker_amount) = demo_counter_order_terms(
                    user_order.claim,
                    user_order.side,
                    U256::from(claim_amount),
                    U256::from(collateral_amount),
                )
                .unwrap();
                let mm_order = Order {
                    salt: U256::from(claim_amount),
                    maker: mm,
                    market_id,
                    claim,
                    maker_amount,
                    taker_amount,
                    side,
                    expiration: U256::ZERO,
                    epoch: U256::ZERO,
                    max_fee_rate_bps: 100,
                };
                let mut book = MarketOrderBook::new(market_id);
                book.insert(B256::repeat_byte(3), user_order).unwrap();

                let plan = plan_match(&book, &mm_order, U256::ZERO, MatchConfig::default())
                    .unwrap()
                    .unwrap();

                assert_eq!(plan.maker_fills.len(), 1);
            }
        }
    }

    #[test]
    fn user_sell_is_matched_with_same_claim_buy() {
        let (claim, side, maker_amount, taker_amount) = demo_counter_order_terms(
            ClaimSide::Payoff,
            Side::Sell,
            U256::from(100),
            U256::from(45),
        )
        .unwrap();

        assert_eq!(claim, ClaimSide::Payoff);
        assert_eq!(side, Side::Buy);
        assert_eq!(maker_amount, U256::from(45));
        assert_eq!(taker_amount, U256::from(100));
    }
}
