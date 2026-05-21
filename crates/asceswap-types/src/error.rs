#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderError {
    ZeroMaker,
    ZeroMarket,
    ZeroAmount,
    ImpossiblePrice,
}
