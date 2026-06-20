pub type OrderId = u64;
pub type SeqNum = u64;
pub type Timestamp = u64;
pub type Price = u64;
pub type Qty = u64;
pub type AccountId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn opposite(self) -> Side {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderType {
    Limit,
    Market,
    Stop { trigger: Price },
    StopLimit { trigger: Price },
    Iceberg { display: Qty },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TimeInForce {
    #[default]
    Gtc,
    Ioc,
    Fok,
    PostOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StpMode {
    #[default]
    Off,
    CancelTaker,
    CancelMaker,
    CancelBoth,
}
