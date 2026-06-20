use std::collections::VecDeque;

use crate::output::Event;
use crate::types::{Price, Qty, SeqNum, Side};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TapeTrade {
    pub seq: SeqNum,
    pub price: Price,
    pub qty: Qty,
    pub taker_side: Side,
}

#[derive(Debug, Default)]
pub struct TradeTape {
    trades: VecDeque<TapeTrade>,
    cap: Option<usize>,
}

impl TradeTape {
    pub fn new() -> Self {
        TradeTape::default()
    }

    pub fn bounded(cap: usize) -> Self {
        TradeTape {
            trades: VecDeque::new(),
            cap: Some(cap),
        }
    }

    pub fn apply(&mut self, events: &[Event]) -> Vec<TapeTrade> {
        let mut out = Vec::new();
        for event in events {
            if let Event::Trade {
                seq,
                price,
                qty,
                taker_side,
                ..
            } = *event
            {
                let trade = TapeTrade {
                    seq,
                    price,
                    qty,
                    taker_side,
                };
                self.record(trade);
                out.push(trade);
            }
        }
        out
    }

    pub fn recent(&self) -> impl Iterator<Item = &TapeTrade> {
        self.trades.iter()
    }

    pub fn last(&self) -> Option<&TapeTrade> {
        self.trades.back()
    }

    pub fn len(&self) -> usize {
        self.trades.len()
    }

    pub fn is_empty(&self) -> bool {
        self.trades.is_empty()
    }

    fn record(&mut self, trade: TapeTrade) {
        self.trades.push_back(trade);
        if let Some(cap) = self.cap
            && self.trades.len() > cap
        {
            self.trades.pop_front();
        }
    }
}
