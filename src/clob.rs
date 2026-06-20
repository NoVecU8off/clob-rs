use crate::book::OrderBook;
use crate::engine::MatchingEngine;
use crate::gateway::Gateway;
use crate::order::Command;
use crate::output::Event;
use crate::sequencer::Sequencer;

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
        }
    }

    pub fn book(&self) -> &OrderBook {
        self.engine.book()
    }

    pub fn current_seq(&self) -> u64 {
        self.sequencer.current_seq()
    }
}
