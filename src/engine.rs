use crate::book::{OrderBook, RestingOrder};
use crate::error::RejectReason;
use crate::order::{ModifyOrder, NewOrder};
use crate::output::Event;
use crate::types::{OrderId, OrderType, SeqNum, TimeInForce, Timestamp};

#[derive(Debug, Default)]
pub struct MatchingEngine {
    book: OrderBook,
}

impl MatchingEngine {
    pub fn new() -> Self {
        MatchingEngine::default()
    }

    pub fn book(&self) -> &OrderBook {
        &self.book
    }

    pub(crate) fn execute_new(
        &mut self,
        seq: SeqNum,
        order_id: OrderId,
        order: NewOrder,
        timestamp: Timestamp,
        out: &mut Vec<Event>,
    ) {
        let limit_price = match order.order_type {
            OrderType::Limit => Some(order.price),
            OrderType::Market => None,
        };

        if order.tif == TimeInForce::Fok
            && self.book.available_qty(order.side, limit_price) < order.qty
        {
            out.push(Event::Rejected {
                seq,
                reason: RejectReason::InsufficientLiquidity,
            });
            return;
        }

        if order.tif == TimeInForce::PostOnly && self.book.would_cross(order.side, limit_price) {
            out.push(Event::Rejected {
                seq,
                reason: RejectReason::WouldCross,
            });
            return;
        }

        out.push(Event::Accepted { seq, order_id });

        let taker_side = order.side;
        let remaining = self.book.match_against(
            taker_side,
            limit_price,
            order.qty,
            |maker, traded, price| {
                out.push(Event::Trade {
                    seq,
                    taker_order_id: order_id,
                    maker_order_id: maker.id,
                    price,
                    qty: traded,
                    taker_side,
                });
            },
        );

        if remaining == 0 {
            out.push(Event::Filled { seq, order_id });
            return;
        }

        let rest_on_book = order.order_type == OrderType::Limit
            && matches!(order.tif, TimeInForce::Gtc | TimeInForce::PostOnly);
        if rest_on_book {
            self.book.insert(
                taker_side,
                RestingOrder {
                    id: order_id,
                    seq,
                    price: order.price,
                    qty: remaining,
                    timestamp,
                },
            );
            out.push(Event::Resting {
                seq,
                order_id,
                price: order.price,
                qty: remaining,
            });
        } else {
            out.push(Event::Canceled { seq, order_id });
        }
    }

    pub(crate) fn execute_cancel(&mut self, seq: SeqNum, order_id: OrderId, out: &mut Vec<Event>) {
        match self.book.cancel(order_id) {
            Some(_) => out.push(Event::Canceled { seq, order_id }),
            None => out.push(Event::Rejected {
                seq,
                reason: RejectReason::UnknownOrder,
            }),
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

        let order_id = modify.order_id;
        let remaining = self.book.match_against(
            side,
            Some(modify.price),
            modify.qty,
            |maker, traded, price| {
                out.push(Event::Trade {
                    seq,
                    taker_order_id: order_id,
                    maker_order_id: maker.id,
                    price,
                    qty: traded,
                    taker_side: side,
                });
            },
        );

        if remaining == 0 {
            out.push(Event::Filled { seq, order_id });
            return;
        }

        self.book.insert(
            side,
            RestingOrder {
                id: order_id,
                seq,
                price: modify.price,
                qty: remaining,
                timestamp,
            },
        );
        out.push(Event::Resting {
            seq,
            order_id,
            price: modify.price,
            qty: remaining,
        });
    }
}
