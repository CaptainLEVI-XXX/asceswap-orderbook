use asceswap_engine::{EngineError, SubmitOrder};
use asceswap_math::{collateral_delta, remaining_claim_amount};
use asceswap_types::{Address, Order, Side, U256};
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

        let side = taker_order.side.opposite();
        let (maker_amount, taker_amount) = match side {
            Side::Buy => (collateral_amount, claim_amount),
            Side::Sell => (claim_amount, collateral_amount),
        };

        let salt = self.next_salt;
        self.next_salt = self
            .next_salt
            .checked_add(1)
            .ok_or(ApiError::SequenceOverflow)?;

        let order = Order {
            salt: U256::from(salt),
            maker: self.maker,
            market_id: taker_order.market_id,
            claim: taker_order.claim,
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
