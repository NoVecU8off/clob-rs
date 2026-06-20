use std::cmp::Reverse;
use std::collections::{BTreeSet, HashMap};

use crate::book::OrderBook;
use crate::clob::Clob;
use crate::output::Event;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct L2Update {
    pub seq: SeqNum,
    pub bids: Vec<L2Level>,
    pub asks: Vec<L2Level>,
}

#[derive(Debug, Default)]
pub struct L2Feed {
    bids: HashMap<Price, Qty>,
    asks: HashMap<Price, Qty>,
    order_loc: HashMap<OrderId, (Side, Price)>,
}

impl L2Feed {
    pub fn new() -> Self {
        L2Feed::default()
    }

    pub fn from_book(book: &OrderBook) -> Self {
        let mut feed = L2Feed::default();
        for (side, order, _) in book.resting_orders() {
            feed.order_loc.insert(order.id, (side, order.price));
            let levels = match side {
                Side::Buy => &mut feed.bids,
                Side::Sell => &mut feed.asks,
            };
            *levels.entry(order.price).or_insert(0) += order.qty;
        }
        feed
    }

    pub fn apply(&mut self, events: &[Event], book: &OrderBook) -> L2Update {
        let mut bids = BTreeSet::new();
        let mut asks = BTreeSet::new();
        {
            let mut touch = |side, price| match side {
                Side::Buy => {
                    bids.insert(price);
                }
                Side::Sell => {
                    asks.insert(price);
                }
            };
            for event in events {
                match *event {
                    Event::Trade {
                        maker_order_id,
                        price,
                        taker_side,
                        ..
                    } => {
                        touch(taker_side.opposite(), price);
                        if book.get(maker_order_id).is_none() {
                            self.order_loc.remove(&maker_order_id);
                        }
                    }
                    Event::Resting {
                        order_id, price, ..
                    } => {
                        if let Some((side, _)) = book.get(order_id) {
                            self.order_loc.insert(order_id, (side, price));
                            touch(side, price);
                        }
                    }
                    Event::Modified { order_id, .. } => {
                        if let Some(&(side, price)) = self.order_loc.get(&order_id) {
                            touch(side, price);
                        }
                    }
                    Event::Canceled { order_id, .. } | Event::Filled { order_id, .. } => {
                        if let Some((side, price)) = self.order_loc.remove(&order_id) {
                            touch(side, price);
                        }
                    }
                    Event::Accepted { .. } | Event::Triggered { .. } | Event::Rejected { .. } => {}
                }
            }
        }
        L2Update {
            seq: command_seq(events),
            bids: self.diff(book, Side::Buy, &bids),
            asks: self.diff(book, Side::Sell, &asks),
        }
    }

    fn diff(&mut self, book: &OrderBook, side: Side, touched: &BTreeSet<Price>) -> Vec<L2Level> {
        let levels = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        let mut out = Vec::new();
        for &price in touched {
            let qty = book.level_qty(side, price);
            if levels.get(&price).copied().unwrap_or(0) == qty {
                continue;
            }
            if qty == 0 {
                levels.remove(&price);
            } else {
                levels.insert(price, qty);
            }
            out.push(L2Level { price, qty });
        }
        match side {
            Side::Buy => out.sort_by_key(|l| Reverse(l.price)),
            Side::Sell => out.sort_by_key(|l| l.price),
        }
        out
    }
}

pub(crate) fn command_seq(events: &[Event]) -> SeqNum {
    events.first().map_or(0, event_seq)
}

fn event_seq(event: &Event) -> SeqNum {
    match *event {
        Event::Accepted { seq, .. }
        | Event::Rejected { seq, .. }
        | Event::Trade { seq, .. }
        | Event::Resting { seq, .. }
        | Event::Filled { seq, .. }
        | Event::Canceled { seq, .. }
        | Event::Modified { seq, .. }
        | Event::Triggered { seq, .. } => seq,
    }
}
