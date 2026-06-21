use crate::error::RejectReason;
use crate::types::{OrderId, Price, Qty, SeqNum, Side};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Accepted {
        seq: SeqNum,
        order_id: OrderId,
    },
    Rejected {
        seq: SeqNum,
        reason: RejectReason,
    },
    Trade {
        seq: SeqNum,
        taker_order_id: OrderId,
        maker_order_id: OrderId,
        price: Price,
        qty: Qty,
        taker_side: Side,
        taker_fee: i64,
        maker_fee: i64,
    },
    Resting {
        seq: SeqNum,
        order_id: OrderId,
        price: Price,
        qty: Qty,
    },
    Filled {
        seq: SeqNum,
        order_id: OrderId,
    },
    Canceled {
        seq: SeqNum,
        order_id: OrderId,
    },
    Modified {
        seq: SeqNum,
        order_id: OrderId,
    },
    Triggered {
        seq: SeqNum,
        order_id: OrderId,
    },
}
