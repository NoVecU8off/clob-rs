use std::collections::{BTreeSet, HashMap};

use crate::book::OrderBook;
use crate::marketdata::command_seq;
use crate::output::Event;
use crate::types::{OrderId, Price, Qty, SeqNum, Side};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum L3Delta {
    Added {
        id: OrderId,
        side: Side,
        price: Price,
        qty: Qty,
    },
    Reduced {
        id: OrderId,
        qty: Qty,
    },
    Removed {
        id: OrderId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct L3Update {
    pub seq: SeqNum,
    pub deltas: Vec<L3Delta>,
}

#[derive(Debug, Default)]
pub struct L3Feed {
    levels: HashMap<(Side, Price), Vec<(OrderId, Qty)>>,
    loc: HashMap<OrderId, (Side, Price)>,
}

impl L3Feed {
    pub fn new() -> Self {
        L3Feed::default()
    }

    pub fn from_book(book: &OrderBook) -> Self {
        let mut feed = L3Feed::default();
        for (side, order, _) in book.resting_orders() {
            feed.loc.insert(order.id, (side, order.price));
            feed.levels
                .entry((side, order.price))
                .or_default()
                .push((order.id, order.qty));
        }
        feed
    }

    pub fn apply(&mut self, events: &[Event], book: &OrderBook) -> L3Update {
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
                        price, taker_side, ..
                    } => touch(taker_side.opposite(), price),
                    Event::Resting {
                        order_id, price, ..
                    } => {
                        if let Some((side, _)) = book.get(order_id) {
                            touch(side, price);
                        }
                    }
                    Event::Modified { order_id, .. }
                    | Event::Canceled { order_id, .. }
                    | Event::Filled { order_id, .. } => {
                        if let Some(&(side, price)) = self.loc.get(&order_id) {
                            touch(side, price);
                        }
                    }
                    Event::Accepted { .. } | Event::Triggered { .. } | Event::Rejected { .. } => {}
                }
            }
        }

        let mut removed = Vec::new();
        let mut reduced = Vec::new();
        let mut added = Vec::new();
        for price in bids.iter().rev().copied().collect::<Vec<_>>() {
            self.diff_level(
                book,
                Side::Buy,
                price,
                &mut removed,
                &mut reduced,
                &mut added,
            );
        }
        for price in asks.iter().copied().collect::<Vec<_>>() {
            self.diff_level(
                book,
                Side::Sell,
                price,
                &mut removed,
                &mut reduced,
                &mut added,
            );
        }

        let mut deltas = removed;
        deltas.extend(reduced);
        deltas.extend(added);
        L3Update {
            seq: command_seq(events),
            deltas,
        }
    }

    fn diff_level(
        &mut self,
        book: &OrderBook,
        side: Side,
        price: Price,
        removed: &mut Vec<L3Delta>,
        reduced: &mut Vec<L3Delta>,
        added: &mut Vec<L3Delta>,
    ) {
        let old = self.levels.remove(&(side, price)).unwrap_or_default();
        let new = book.level_orders(side, price);

        let mut keep = 0;
        let mut oi = 0;
        for (id, _) in &new {
            while oi < old.len() && old[oi].0 != *id {
                oi += 1;
            }
            if oi < old.len() {
                oi += 1;
                keep += 1;
            } else {
                break;
            }
        }

        let survivors: BTreeSet<OrderId> = new[..keep].iter().map(|(id, _)| *id).collect();
        let old_qty: HashMap<OrderId, Qty> = old.iter().copied().collect();
        for (id, _) in &old {
            if !survivors.contains(id) {
                removed.push(L3Delta::Removed { id: *id });
                self.loc.remove(id);
            }
        }
        for (id, qty) in &new[..keep] {
            if old_qty.get(id) != Some(qty) {
                reduced.push(L3Delta::Reduced { id: *id, qty: *qty });
            }
        }
        for (id, qty) in &new[keep..] {
            added.push(L3Delta::Added {
                id: *id,
                side,
                price,
                qty: *qty,
            });
            self.loc.insert(*id, (side, price));
        }

        if !new.is_empty() {
            self.levels.insert((side, price), new);
        }
    }
}
