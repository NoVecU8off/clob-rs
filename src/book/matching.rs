use std::collections::HashMap;

use crate::slab::{PriceLevel, Slab};
use crate::types::{AccountId, OrderId, Price, Qty, Side, StpMode};

use super::accounts::AccountBook;
use super::{Location, MatchOutcome, OrderBook, Reserve, RestingOrder};

impl OrderBook {
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
        let maker_side = taker_side.opposite();
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
                                &mut self.accounts,
                                maker_side,
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
                                &mut self.accounts,
                                maker_side,
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
                self.accounts.fill(taker_owner, taker_side, traded);
                self.accounts.fill(fill.owner, maker_side, traded);
                self.accounts.sub_open(fill.owner, maker_side, traded);
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
        accounts: &mut AccountBook,
        maker_side: Side,
        level: &mut PriceLevel,
        slot: u32,
    ) -> OrderId {
        let (id, qty, owner) = {
            let order = &slab.nodes[slot as usize].order;
            (order.id, order.qty, order.owner)
        };
        level.total_qty -= qty;
        level.unlink(slab, slot);
        index.remove(&id);
        let hidden = reserves.remove(&id).map_or(0, |r| r.hidden);
        accounts.sub_open(owner, maker_side, qty + hidden);
        slab.dealloc(slot);
        id
    }
}
