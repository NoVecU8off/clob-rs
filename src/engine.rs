use crate::book::{OrderBook, RestingEntry, RestingOrder};
use crate::error::RejectReason;
use crate::fees::FeeConfig;
use crate::order::{ModifyOrder, NewOrder};
use crate::output::Event;
use crate::stops::{PendingStop, StopBook};
use crate::types::{
    AccountId, OrderId, OrderType, Price, Qty, SeqNum, Side, StpMode, TimeInForce, Timestamp,
};

struct Live {
    seq: SeqNum,
    order_id: OrderId,
    side: Side,
    limit_price: Option<Price>,
    qty: Qty,
    tif: TimeInForce,
    timestamp: Timestamp,
    display: Qty,
    owner: AccountId,
    stp: StpMode,
}

#[derive(Debug, Default)]
pub struct MatchingEngine {
    book: OrderBook,
    stops: StopBook,
    last_trade_price: Option<Price>,
    fees: FeeConfig,
}

impl MatchingEngine {
    pub fn new() -> Self {
        MatchingEngine::default()
    }

    pub fn book(&self) -> &OrderBook {
        &self.book
    }

    pub fn pending_stops(&self) -> usize {
        self.stops.len()
    }

    pub(crate) fn last_trade_price(&self) -> Option<Price> {
        self.last_trade_price
    }

    pub(crate) fn resting_orders(&self) -> Vec<RestingEntry> {
        self.book.resting_orders()
    }

    pub(crate) fn pending_stops_data(&self) -> Vec<PendingStop> {
        self.stops.pending().to_vec()
    }

    pub(crate) fn restore_last_trade_price(&mut self, price: Option<Price>) {
        self.last_trade_price = price;
    }

    pub(crate) fn set_fees(&mut self, fees: FeeConfig) {
        self.fees = fees;
    }

    pub(crate) fn restore_order(
        &mut self,
        side: Side,
        order: RestingOrder,
        reserve: Option<(Qty, Qty)>,
    ) {
        let id = order.id;
        self.book.insert(side, order);
        if let Some((display, hidden)) = reserve {
            self.book.add_reserve(id, display, hidden);
        }
    }

    pub(crate) fn restore_stop(&mut self, stop: PendingStop) {
        self.stops.park(stop);
    }

    pub(crate) fn restore_account_net(&mut self, owner: AccountId, net: i128) {
        self.book.restore_account_net(owner, net);
    }

    pub(crate) fn execute_new(
        &mut self,
        seq: SeqNum,
        order_id: OrderId,
        order: NewOrder,
        timestamp: Timestamp,
        out: &mut Vec<Event>,
    ) {
        match order.order_type {
            OrderType::Stop { trigger } | OrderType::StopLimit { trigger } => {
                out.push(Event::Accepted { seq, order_id });
                let (activates_to, limit_price) = match order.order_type {
                    OrderType::StopLimit { .. } => (OrderType::Limit, order.price),
                    _ => (OrderType::Market, 0),
                };
                self.stops.park(PendingStop {
                    id: order_id,
                    side: order.side,
                    trigger,
                    activates_to,
                    limit_price,
                    qty: order.qty,
                    tif: order.tif,
                    owner: order.owner,
                    stp: order.stp,
                });
                self.drive_stops(seq, timestamp, out);
            }
            OrderType::Limit | OrderType::Market | OrderType::Iceberg { .. } => {
                let (limit_price, display) = match order.order_type {
                    OrderType::Limit => (Some(order.price), 0),
                    OrderType::Iceberg { display } => (Some(order.price), display),
                    _ => (None, 0),
                };
                if let Err(reason) = self.precheck(order.side, limit_price, order.qty, order.tif) {
                    out.push(Event::Rejected { seq, reason });
                    return;
                }
                out.push(Event::Accepted { seq, order_id });
                self.settle(
                    Live {
                        seq,
                        order_id,
                        side: order.side,
                        limit_price,
                        qty: order.qty,
                        tif: order.tif,
                        timestamp,
                        display,
                        owner: order.owner,
                        stp: order.stp,
                    },
                    out,
                );
                self.drive_stops(seq, timestamp, out);
            }
        }
    }

    pub(crate) fn execute_cancel(&mut self, seq: SeqNum, order_id: OrderId, out: &mut Vec<Event>) {
        if self.book.cancel(order_id).is_some() || self.stops.cancel(order_id).is_some() {
            out.push(Event::Canceled { seq, order_id });
        } else {
            out.push(Event::Rejected {
                seq,
                reason: RejectReason::UnknownOrder,
            });
        }
    }

    pub(crate) fn execute_modify(
        &mut self,
        seq: SeqNum,
        modify: ModifyOrder,
        timestamp: Timestamp,
        out: &mut Vec<Event>,
    ) {
        let Some((side, current)) = self.book.get(modify.order_id) else {
            out.push(Event::Rejected {
                seq,
                reason: RejectReason::UnknownOrder,
            });
            return;
        };

        out.push(Event::Modified {
            seq,
            order_id: modify.order_id,
        });

        if let Some((display, _)) = self.book.reserve(modify.order_id) {
            self.book.cancel(modify.order_id);
            self.settle(
                Live {
                    seq,
                    order_id: modify.order_id,
                    side,
                    limit_price: Some(modify.price),
                    qty: modify.qty,
                    tif: TimeInForce::Gtc,
                    timestamp,
                    display,
                    owner: current.owner,
                    stp: StpMode::Off,
                },
                out,
            );
            self.drive_stops(seq, timestamp, out);
            return;
        }

        if modify.price == current.price && modify.qty <= current.qty {
            if modify.qty < current.qty {
                self.book.reduce(modify.order_id, modify.qty);
            }
            out.push(Event::Resting {
                seq,
                order_id: modify.order_id,
                price: modify.price,
                qty: modify.qty,
            });
            return;
        }

        self.book.cancel(modify.order_id);
        self.settle(
            Live {
                seq,
                order_id: modify.order_id,
                side,
                limit_price: Some(modify.price),
                qty: modify.qty,
                tif: TimeInForce::Gtc,
                timestamp,
                display: 0,
                owner: current.owner,
                stp: StpMode::Off,
            },
            out,
        );
        self.drive_stops(seq, timestamp, out);
    }

    fn precheck(
        &self,
        side: Side,
        limit_price: Option<Price>,
        qty: Qty,
        tif: TimeInForce,
    ) -> Result<(), RejectReason> {
        if tif == TimeInForce::Fok && self.book.available_qty(side, limit_price) < qty {
            return Err(RejectReason::InsufficientLiquidity);
        }
        if tif == TimeInForce::PostOnly && self.book.would_cross(side, limit_price) {
            return Err(RejectReason::WouldCross);
        }
        Ok(())
    }

    fn settle(&mut self, live: Live, out: &mut Vec<Event>) {
        let Live {
            seq,
            order_id,
            side,
            limit_price,
            qty,
            tif,
            timestamp,
            display,
            owner,
            stp,
        } = live;

        let mut last_px = None;
        let fees = self.fees;
        let outcome = self.book.match_against(
            side,
            owner,
            stp,
            limit_price,
            qty,
            |maker, traded, price| {
                out.push(Event::Trade {
                    seq,
                    taker_order_id: order_id,
                    maker_order_id: maker.id,
                    price,
                    qty: traded,
                    taker_side: side,
                    taker_fee: fees.taker_fee(price, traded),
                    maker_fee: fees.maker_fee(price, traded),
                });
                last_px = Some(price);
            },
        );
        if let Some(price) = last_px {
            self.last_trade_price = Some(price);
        }
        for maker_id in outcome.self_canceled {
            out.push(Event::Canceled {
                seq,
                order_id: maker_id,
            });
        }

        if outcome.taker_canceled {
            out.push(Event::Canceled { seq, order_id });
            return;
        }

        let remaining = outcome.remaining;
        if remaining == 0 {
            out.push(Event::Filled { seq, order_id });
            return;
        }

        if let Some(price) = limit_price
            && matches!(tif, TimeInForce::Gtc | TimeInForce::PostOnly)
        {
            let (visible, hidden) = if display > 0 && display < remaining {
                (display, remaining - display)
            } else {
                (remaining, 0)
            };
            self.book.insert(
                side,
                RestingOrder {
                    id: order_id,
                    seq,
                    price,
                    qty: visible,
                    timestamp,
                    owner,
                },
            );
            if hidden > 0 {
                self.book.add_reserve(order_id, display, hidden);
            }
            out.push(Event::Resting {
                seq,
                order_id,
                price,
                qty: visible,
            });
            return;
        }

        out.push(Event::Canceled { seq, order_id });
    }

    fn drive_stops(&mut self, seq: SeqNum, timestamp: Timestamp, out: &mut Vec<Event>) {
        while let Some(last) = self.last_trade_price {
            let Some(stop) = self.stops.take_triggered(last) else {
                break;
            };
            out.push(Event::Triggered {
                seq,
                order_id: stop.id,
            });
            let limit_price = match stop.activates_to {
                OrderType::Limit => Some(stop.limit_price),
                _ => None,
            };
            if let Err(reason) = self.precheck(stop.side, limit_price, stop.qty, stop.tif) {
                out.push(Event::Rejected { seq, reason });
                continue;
            }
            self.settle(
                Live {
                    seq,
                    order_id: stop.id,
                    side: stop.side,
                    limit_price,
                    qty: stop.qty,
                    tif: stop.tif,
                    timestamp,
                    display: 0,
                    owner: stop.owner,
                    stp: stop.stp,
                },
                out,
            );
        }
    }
}
