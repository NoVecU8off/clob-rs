use std::cmp::Reverse;

use crate::clob::Clob;
use crate::types::{OrderId, Price, Qty, SeqNum, Side};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct L2Level {
    pub price: Price,
    pub qty: Qty,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct L2Snapshot {
    pub seq: SeqNum,
    pub bids: Vec<L2Level>,
    pub asks: Vec<L2Level>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct L3Order {
    pub id: OrderId,
    pub side: Side,
    pub price: Price,
    pub qty: Qty,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct L3Snapshot {
    pub seq: SeqNum,
    pub bids: Vec<L3Order>,
    pub asks: Vec<L3Order>,
}

impl Clob {
    pub fn l2_snapshot(&self, depth: usize) -> L2Snapshot {
        let book = self.book();
        L2Snapshot {
            seq: self.current_seq(),
            bids: book
                .depth(Side::Buy, depth)
                .into_iter()
                .map(|(price, qty)| L2Level { price, qty })
                .collect(),
            asks: book
                .depth(Side::Sell, depth)
                .into_iter()
                .map(|(price, qty)| L2Level { price, qty })
                .collect(),
        }
    }

    pub fn l3_snapshot(&self) -> L3Snapshot {
        let mut bids = Vec::new();
        let mut asks = Vec::new();
        for (side, order, _) in self.book().resting_orders() {
            let entry = L3Order {
                id: order.id,
                side,
                price: order.price,
                qty: order.qty,
            };
            match side {
                Side::Buy => bids.push(entry),
                Side::Sell => asks.push(entry),
            }
        }
        bids.sort_by_key(|o| Reverse(o.price));
        asks.sort_by_key(|o| o.price);
        L3Snapshot {
            seq: self.current_seq(),
            bids,
            asks,
        }
    }
}
