use asceswap_engine::EngineError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageError {
    DuplicateEventSequence(u64),
    DuplicateFillSequence(u64),
    MissingSnapshot,
    Backend(String),
    SequenceOverflow,
    Recovery(EngineError),
}

impl StorageError {
    pub fn backend(error: impl ToString) -> Self {
        Self::Backend(error.to_string())
    }
}
