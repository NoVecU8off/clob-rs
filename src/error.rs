#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RejectReason {
    ZeroQuantity,
    InvalidPrice,
    UnknownOrder,
    InsufficientLiquidity,
    WouldCross,
    TickSize,
    LotSize,
    PriceBand,
    PositionLimit,
}
