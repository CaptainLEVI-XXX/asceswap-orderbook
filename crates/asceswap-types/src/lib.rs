pub use alloy_primitives::{Address, B256, U256, U512};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ClaimSide {
    Residual,
    Payoff,
}

impl ClaimSide {
    pub fn opposite(self) -> Self {
        match self {
            Self::Residual => Self::Payoff,
            Self::Payoff => Self::Residual,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn opposite(self) -> Self {
        match self {
            Self::Buy => Self::Sell,
            Self::Sell => Self::Buy,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MatchKind {
    Direct,
    MintAssisted,
    MergeAssisted,
}

pub type MarketId = B256;
pub type OrderHash = B256;
pub type Amount = U256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Order {
    pub salt: U256,
    pub maker: Address,
    pub market_id: MarketId,
    pub claim: ClaimSide,
    pub maker_amount: U256,
    pub taker_amount: U256,
    pub side: Side,
    pub expiration: U256,
    pub epoch: U256,
    pub max_fee_rate_bps: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderError {
    ZeroMaker,
    ZeroMarket,
    ZeroAmount,
    ImpossiblePrice,
}

impl Order {
    pub fn validate_basic(&self) -> Result<(), OrderError> {
        if self.maker == Address::ZERO {
            return Err(OrderError::ZeroMaker);
        }

        if self.market_id == B256::ZERO {
            return Err(OrderError::ZeroMarket);
        }

        if self.maker_amount == U256::ZERO || self.taker_amount == U256::ZERO {
            return Err(OrderError::ZeroAmount);
        }

        match self.side {
            Side::Buy if self.maker_amount > self.taker_amount => Err(OrderError::ImpossiblePrice),
            Side::Sell if self.taker_amount > self.maker_amount => Err(OrderError::ImpossiblePrice),
            _ => Ok(()),
        }
    }

    pub fn max_claim_amount(&self) -> U256 {
        match self.side {
            Side::Buy => self.taker_amount,
            Side::Sell => self.maker_amount,
        }
    }

    pub fn collateral_ratio_parts(&self) -> (U256, U256) {
        match self.side {
            Side::Buy => (self.maker_amount, self.taker_amount),
            Side::Sell => (self.taker_amount, self.maker_amount),
        }
    }
}

