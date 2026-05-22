use asceswap_engine::EngineError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageError {
    DuplicateEventSequence(u64),
    DuplicateFillSequence(u64),
    MissingSnapshot,
    SequenceOverflow,
    Recovery(EngineError),
}
