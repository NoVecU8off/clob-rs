use crate::book::OrderBook;
use crate::engine::MatchingEngine;
use crate::gateway::Gateway;
use crate::order::Command;
use crate::output::Event;
use crate::sequencer::Sequencer;
use crate::snapshot::SnapshotState;

#[derive(Debug, Default)]
pub struct Clob {
    gateway: Gateway,
    sequencer: Sequencer,
    engine: MatchingEngine,
}

impl Clob {
    pub fn new() -> Self {
        Clob::default()
    }

    pub fn submit(&mut self, command: Command) -> Vec<Event> {
        let mut out = Vec::new();
        self.submit_into(command, &mut out);
        out
    }

    pub fn submit_into(&mut self, command: Command, out: &mut Vec<Event>) {
        let seq = self.sequencer.next_seq();

        if let Err(reason) = self.gateway.validate(&command) {
            out.push(Event::Rejected { seq, reason });
            return;
        }

        match command {
            Command::New(order) => {
                let order_id = self.sequencer.next_order_id();
                self.engine.execute_new(seq, order_id, order, seq, out);
            }
            Command::Cancel(cancel) => {
                self.engine.execute_cancel(seq, cancel.order_id, out);
            }
            Command::Modify(modify) => {
                self.engine.execute_modify(seq, modify, seq, out);
            }
        }
    }

    pub fn book(&self) -> &OrderBook {
        self.engine.book()
    }

    pub fn pending_stops(&self) -> usize {
        self.engine.pending_stops()
    }

    pub fn current_seq(&self) -> u64 {
        self.sequencer.current_seq()
    }

    pub(crate) fn capture(&self) -> SnapshotState {
        let (seq, next_order_id) = self.sequencer.snapshot();
        SnapshotState {
            seq,
            next_order_id,
            last_trade_price: self.engine.last_trade_price(),
            orders: self.engine.resting_orders(),
            stops: self.engine.pending_stops_data(),
        }
    }

    pub(crate) fn from_snapshot(state: SnapshotState) -> Self {
        let mut engine = MatchingEngine::new();
        engine.restore_last_trade_price(state.last_trade_price);
        for (side, order, reserve) in state.orders {
            engine.restore_order(side, order, reserve);
        }
        for stop in state.stops {
            engine.restore_stop(stop);
        }
        Clob {
            gateway: Gateway::new(),
            sequencer: Sequencer::restore(state.seq, state.next_order_id),
            engine,
        }
    }
}
