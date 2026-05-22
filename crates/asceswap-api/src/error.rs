use asceswap_engine::EngineError;
use asceswap_storage::StorageError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiError {
    InvalidField {
        field: &'static str,
        reason: &'static str,
    },
    OrderNotFound(String),
    SequenceOverflow,
    Engine(EngineError),
    Storage(StorageError),
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
