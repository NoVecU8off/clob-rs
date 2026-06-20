use crate::types::{OrderId, OrderType, Price, Qty, Side, TimeInForce};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    New(NewOrder),
    Cancel(CancelOrder),
    Modify(ModifyOrder),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NewOrder {
    pub side: Side,
    pub order_type: OrderType,
    pub price: Price,
    pub qty: Qty,
    pub tif: TimeInForce,
}

impl NewOrder {
    pub fn limit(side: Side, price: Price, qty: Qty) -> Self {
        NewOrder {
            side,
            order_type: OrderType::Limit,
            price,
            qty,
            tif: TimeInForce::Gtc,
        }
    }

    pub fn market(side: Side, qty: Qty) -> Self {
        NewOrder {
            side,
            order_type: OrderType::Market,
            price: 0,
            qty,
            tif: TimeInForce::Ioc,
        }
    }

    pub fn with_tif(mut self, tif: TimeInForce) -> Self {
        self.tif = tif;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CancelOrder {
    pub order_id: OrderId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModifyOrder {
    pub order_id: OrderId,
    pub price: Price,
    pub qty: Qty,
}

impl ModifyOrder {
    pub fn new(order_id: OrderId, price: Price, qty: Qty) -> Self {
        ModifyOrder {
            order_id,
            price,
            qty,
        }
    }
}

impl From<NewOrder> for Command {
    fn from(order: NewOrder) -> Self {
        Command::New(order)
    }
}

impl From<CancelOrder> for Command {
    fn from(cancel: CancelOrder) -> Self {
        Command::Cancel(cancel)
    }
}

impl From<ModifyOrder> for Command {
    fn from(modify: ModifyOrder) -> Self {
        Command::Modify(modify)
    }
}
