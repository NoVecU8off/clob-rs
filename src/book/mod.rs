mod accounts;
mod matching;

use std::collections::{BTreeMap, HashMap};

use crate::slab::{Node, PriceLevel, Slab};
use crate::types::{AccountId, OrderId, Price, Qty, SeqNum, Side, Timestamp};

use accounts::AccountBook;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestingOrder {
    pub id: OrderId,
    pub seq: SeqNum,
    pub price: Price,
    pub qty: Qty,
    pub timestamp: Timestamp,
    pub owner: AccountId,
}

pub(crate) type RestingEntry = (Side, RestingOrder, Option<(Qty, Qty)>);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchOutcome {
    pub remaining: Qty,
    pub taker_canceled: bool,
    pub self_canceled: Vec<OrderId>,
}

#[derive(Clone, Copy, Debug)]
struct Location {
    side: Side,
    slot: u32,
}

#[derive(Clone, Copy, Debug)]
struct Reserve {
    display: Qty,
    hidden: Qty,
}

#[derive(Debug, Default)]
pub struct OrderBook {
    bids: BTreeMap<Price, PriceLevel>,
    asks: BTreeMap<Price, PriceLevel>,
    slab: Slab,
    index: HashMap<OrderId, Location>,
    reserves: HashMap<OrderId, Reserve>,
    accounts: AccountBook,
}

impl OrderBook {
    pub fn new() -> Self {
        OrderBook::default()
    }

    pub fn best_bid(&self) -> Option<Price> {
        self.bids.keys().next_back().copied()
    }

    pub fn best_ask(&self) -> Option<Price> {
        self.asks.keys().next().copied()
    }

    pub fn spread(&self) -> Option<Price> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some(ask.saturating_sub(bid)),
            _ => None,
        }
    }

    pub fn mid(&self) -> Option<Price> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some(((bid as u128 + ask as u128) / 2) as Price),
            _ => None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn contains(&self, order_id: OrderId) -> bool {
        self.index.contains_key(&order_id)
    }

    pub(crate) fn account_net(&self, owner: AccountId) -> i128 {
        self.accounts.net(owner)
    }

    pub(crate) fn account_open(&self, owner: AccountId, side: Side) -> Qty {
        self.accounts.open(owner, side)
    }

    pub(crate) fn account_nets(&self) -> Vec<(AccountId, i128)> {
        self.accounts.nets_sorted()
    }

    pub(crate) fn restore_account_net(&mut self, owner: AccountId, net: i128) {
        self.accounts.restore_net(owner, net);
    }

    pub fn depth(&self, side: Side, levels: usize) -> Vec<(Price, Qty)> {
        match side {
            Side::Buy => self
                .bids
                .iter()
                .rev()
                .take(levels)
                .map(|(price, level)| (*price, level.total_qty))
                .collect(),
            Side::Sell => self
                .asks
                .iter()
                .take(levels)
                .map(|(price, level)| (*price, level.total_qty))
                .collect(),
        }
    }

    pub fn level_qty(&self, side: Side, price: Price) -> Qty {
        let book = match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        };
        book.get(&price).map_or(0, |level| level.total_qty)
    }

    pub fn level_orders(&self, side: Side, price: Price) -> Vec<(OrderId, Qty)> {
        let book = match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        };
        let mut out = Vec::new();
        if let Some(level) = book.get(&price) {
            let mut cursor = level.head;
            while let Some(slot) = cursor {
                let node = self.slab.nodes[slot as usize];
                out.push((node.order.id, node.order.qty));
                cursor = node.next;
            }
        }
        out
    }

    pub fn available_qty(&self, taker_side: Side, limit_price: Option<Price>) -> Qty {
        let mut total: Qty = 0;
        match taker_side {
            Side::Buy => {
                for (price, level) in self.asks.iter() {
                    if let Some(limit) = limit_price
                        && limit < *price
                    {
                        break;
                    }
                    total += level.total_qty;
                }
            }
            Side::Sell => {
                for (price, level) in self.bids.iter().rev() {
                    if let Some(limit) = limit_price
                        && limit > *price
                    {
                        break;
                    }
                    total += level.total_qty;
                }
            }
        }
        total
    }

    pub fn would_cross(&self, taker_side: Side, limit_price: Option<Price>) -> bool {
        match (taker_side, limit_price) {
            (_, None) => true,
            (Side::Buy, Some(limit)) => self.best_ask().is_some_and(|ask| limit >= ask),
            (Side::Sell, Some(limit)) => self.best_bid().is_some_and(|bid| limit <= bid),
        }
    }

    pub fn insert(&mut self, side: Side, order: RestingOrder) {
        let slot = self.slab.alloc(Node {
            order,
            prev: None,
            next: None,
        });
        self.index.insert(order.id, Location { side, slot });
        let book = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        let level = book.entry(order.price).or_default();
        level.total_qty += order.qty;
        level.link_back(&mut self.slab, slot);
        self.accounts.add_open(order.owner, side, order.qty);
    }

    pub fn add_reserve(&mut self, order_id: OrderId, display: Qty, hidden: Qty) {
        if let Some((side, order)) = self.get(order_id) {
            self.accounts.add_open(order.owner, side, hidden);
        }
        self.reserves.insert(order_id, Reserve { display, hidden });
    }

    pub fn reserve(&self, order_id: OrderId) -> Option<(Qty, Qty)> {
        self.reserves.get(&order_id).map(|r| (r.display, r.hidden))
    }

    pub(crate) fn resting_orders(&self) -> Vec<RestingEntry> {
        let mut out = Vec::with_capacity(self.index.len());
        for (side, levels) in [(Side::Buy, &self.bids), (Side::Sell, &self.asks)] {
            for level in levels.values() {
                let mut cursor = level.head;
                while let Some(slot) = cursor {
                    let node = self.slab.nodes[slot as usize];
                    out.push((side, node.order, self.reserve(node.order.id)));
                    cursor = node.next;
                }
            }
        }
        out
    }

    pub fn cancel(&mut self, order_id: OrderId) -> Option<RestingOrder> {
        let location = self.index.remove(&order_id)?;
        let order = self.slab.nodes[location.slot as usize].order;
        let book = match location.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        let level = book.get_mut(&order.price)?;
        level.total_qty -= order.qty;
        level.unlink(&mut self.slab, location.slot);
        if level.head.is_none() {
            book.remove(&order.price);
        }
        self.slab.dealloc(location.slot);
        let hidden = self.reserves.remove(&order_id).map_or(0, |r| r.hidden);
        self.accounts
            .sub_open(order.owner, location.side, order.qty + hidden);
        Some(order)
    }

    pub fn get(&self, order_id: OrderId) -> Option<(Side, RestingOrder)> {
        let location = self.index.get(&order_id)?;
        Some((location.side, self.slab.nodes[location.slot as usize].order))
    }

    pub fn reduce(&mut self, order_id: OrderId, new_qty: Qty) -> Option<RestingOrder> {
        let location = *self.index.get(&order_id)?;
        let (price, delta, updated) = {
            let node = &mut self.slab.nodes[location.slot as usize];
            if new_qty >= node.order.qty {
                return None;
            }
            let delta = node.order.qty - new_qty;
            node.order.qty = new_qty;
            (node.order.price, delta, node.order)
        };
        let book = match location.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        if let Some(level) = book.get_mut(&price) {
            level.total_qty -= delta;
        }
        self.accounts.sub_open(updated.owner, location.side, delta);
        Some(updated)
    }
}
