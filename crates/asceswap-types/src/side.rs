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
