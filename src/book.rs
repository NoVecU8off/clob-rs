use std::collections::{BTreeMap, HashMap, VecDeque};

use crate::types::{OrderId, Price, Qty, SeqNum, Side, Timestamp};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RestingOrder {
    pub id: OrderId,
    pub seq: SeqNum,
    pub price: Price,
    pub qty: Qty,
    pub timestamp: Timestamp,
}

#[derive(Debug, Default)]
struct PriceLevel {
    orders: VecDeque<RestingOrder>,
    total_qty: Qty,
}

impl PriceLevel {
    fn push(&mut self, order: RestingOrder) {
        self.total_qty += order.qty;
        self.orders.push_back(order);
    }
}

#[derive(Clone, Copy, Debug)]
struct Location {
    side: Side,
    price: Price,
}

#[derive(Debug, Default)]
pub struct OrderBook {
    bids: BTreeMap<Price, PriceLevel>,
    asks: BTreeMap<Price, PriceLevel>,
    index: HashMap<OrderId, Location>,
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

    pub fn insert(&mut self, side: Side, order: RestingOrder) {
        self.index.insert(
            order.id,
            Location {
                side,
                price: order.price,
            },
        );
        let book = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        book.entry(order.price).or_default().push(order);
    }

    pub fn cancel(&mut self, order_id: OrderId) -> Option<RestingOrder> {
        let location = self.index.remove(&order_id)?;
        let book = match location.side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        let level = book.get_mut(&location.price)?;
        let position = level.orders.iter().position(|o| o.id == order_id)?;
        let removed = level.orders.remove(position)?;
        level.total_qty -= removed.qty;
        if level.orders.is_empty() {
            book.remove(&location.price);
        }
        Some(removed)
    }

    pub fn match_against<F>(
        &mut self,
        taker_side: Side,
        limit_price: Option<Price>,
        mut qty: Qty,
        mut on_trade: F,
    ) -> Qty
    where
        F: FnMut(&RestingOrder, Qty, Price),
    {
        let mut filled_ids: Vec<OrderId> = Vec::new();
        {
            let book = match taker_side {
                Side::Buy => &mut self.asks,
                Side::Sell => &mut self.bids,
            };

            while qty > 0 {
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
                while qty > 0 {
                    let Some(front) = level.orders.front_mut() else {
                        break;
                    };
                    let traded = qty.min(front.qty);
                    front.qty -= traded;
                    level.total_qty -= traded;
                    qty -= traded;

                    let fill = RestingOrder {
                        qty: traded,
                        ..*front
                    };
                    on_trade(&fill, traded, best_price);

                    if front.qty == 0 {
                        let done = level.orders.pop_front().expect("front order exists");
                        filled_ids.push(done.id);
                    }
                }

                if level.orders.is_empty() {
                    book.remove(&best_price);
                }
            }
        }

        for id in filled_ids {
            self.index.remove(&id);
        }
        qty
    }
}
