use asceswap_engine::EngineError;
use asceswap_market_actor::MarketActorError;
use asceswap_storage::StorageError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiError {
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
    OrderNotFound(String),
    SequenceOverflow,
    ServiceClosed,
    ServiceInboxCapacityZero,
    Actor(MarketActorError),
    Engine(EngineError),
    Storage(StorageError),
}

impl From<MarketActorError> for ApiError {
    fn from(error: MarketActorError) -> Self {
        Self::Actor(error)
    }
}

impl From<EngineError> for ApiError {
    fn from(error: EngineError) -> Self {
        Self::Engine(error)
    }
}

impl From<StorageError> for ApiError {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}
