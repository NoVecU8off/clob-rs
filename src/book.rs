use std::collections::{BTreeMap, HashMap};

use crate::slab::{Node, PriceLevel, Slab};
use crate::types::{AccountId, OrderId, Price, Qty, SeqNum, Side, StpMode, Timestamp};

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

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn contains(&self, order_id: OrderId) -> bool {
        self.index.contains_key(&order_id)
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
    }

    pub fn add_reserve(&mut self, order_id: OrderId, display: Qty, hidden: Qty) {
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
        self.reserves.remove(&order_id);
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
        Some(updated)
    }

    pub fn match_against<F>(
        &mut self,
        taker_side: Side,
        taker_owner: AccountId,
        taker_stp: StpMode,
        limit_price: Option<Price>,
        mut qty: Qty,
        mut on_trade: F,
    ) -> MatchOutcome
    where
        F: FnMut(&RestingOrder, Qty, Price),
    {
        let mut taker_canceled = false;
        let mut self_canceled = Vec::new();
        while qty > 0 {
            let book = match taker_side {
                Side::Buy => &mut self.asks,
                Side::Sell => &mut self.bids,
            };
            let best_price = match taker_side {
                Side::Buy => book.keys().next().copied(),
                Side::Sell => book.keys().next_back().copied(),
            };
            let Some(best_price) = best_price else {
                break;
            };

            let crosses = match (taker_side, limit_price) {
                (_, None) => true,
                (Side::Buy, Some(limit)) => limit >= best_price,
                (Side::Sell, Some(limit)) => limit <= best_price,
            };
            if !crosses {
                break;
            }

            let level = book.get_mut(&best_price).expect("price level must exist");
            let mut stop = false;
            while qty > 0 {
                let Some(slot) = level.head else {
                    break;
                };
                if taker_stp != StpMode::Off
                    && taker_owner != 0
                    && self.slab.nodes[slot as usize].order.owner == taker_owner
                {
                    match taker_stp {
                        StpMode::CancelMaker => {
                            let id = Self::detach(
                                &mut self.slab,
                                &mut self.index,
                                &mut self.reserves,
                                level,
                                slot,
                            );
                            self_canceled.push(id);
                            continue;
                        }
                        StpMode::CancelBoth => {
                            let id = Self::detach(
                                &mut self.slab,
                                &mut self.index,
                                &mut self.reserves,
                                level,
                                slot,
                            );
                            self_canceled.push(id);
                            taker_canceled = true;
                            stop = true;
                            break;
                        }
                        StpMode::CancelTaker => {
                            taker_canceled = true;
                            stop = true;
                            break;
                        }
                        StpMode::Off => {}
                    }
                }
                let (traded, fill, filled) = {
                    let node = &mut self.slab.nodes[slot as usize];
                    let traded = qty.min(node.order.qty);
                    node.order.qty -= traded;
                    let fill = RestingOrder {
                        qty: traded,
                        ..node.order
                    };
                    (traded, fill, node.order.qty == 0)
                };
                qty -= traded;
                level.total_qty -= traded;
                on_trade(&fill, traded, best_price);
                if filled {
                    let refill = if self.reserves.is_empty() {
                        None
                    } else if let Some(reserve) = self.reserves.get_mut(&fill.id) {
                        let peak = reserve.display.min(reserve.hidden);
                        reserve.hidden -= peak;
                        if reserve.hidden == 0 {
                            self.reserves.remove(&fill.id);
                        }
                        Some(peak)
                    } else {
                        None
                    };
                    match refill {
                        Some(peak) => {
                            self.slab.nodes[slot as usize].order.qty = peak;
                            level.total_qty += peak;
                            level.unlink(&mut self.slab, slot);
                            level.link_back(&mut self.slab, slot);
                        }
                        None => {
                            self.index.remove(&fill.id);
                            level.unlink(&mut self.slab, slot);
                            self.slab.dealloc(slot);
                        }
                    }
                }
            }

            if level.head.is_none() {
                book.remove(&best_price);
            }
            if stop {
                break;
            }
        }
        MatchOutcome {
            remaining: qty,
            taker_canceled,
            self_canceled,
        }
    }

    fn detach(
        slab: &mut Slab,
        index: &mut HashMap<OrderId, Location>,
        reserves: &mut HashMap<OrderId, Reserve>,
        level: &mut PriceLevel,
        slot: u32,
    ) -> OrderId {
        let (id, qty) = {
            let order = &slab.nodes[slot as usize].order;
            (order.id, order.qty)
        };
        level.total_qty -= qty;
        level.unlink(slab, slot);
        index.remove(&id);
        reserves.remove(&id);
        slab.dealloc(slot);
        id
    }
}
