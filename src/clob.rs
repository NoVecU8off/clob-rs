use crate::book::OrderBook;
use crate::engine::MatchingEngine;
use crate::fees::FeeConfig;
use crate::gateway::{Gateway, RiskContext};
use crate::order::Command;
use crate::output::Event;
use crate::risk::RiskConfig;
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

    pub fn with_risk(risk: RiskConfig) -> Self {
        Clob {
            gateway: Gateway::with_config(risk),
            ..Default::default()
        }
    }

    pub fn with_fees(fees: FeeConfig) -> Self {
        let mut clob = Clob::default();
        clob.engine.set_fees(fees);
        clob
    }

    pub fn with_risk_and_fees(risk: RiskConfig, fees: FeeConfig) -> Self {
        let mut clob = Clob {
            gateway: Gateway::with_config(risk),
            ..Default::default()
        };
        clob.engine.set_fees(fees);
        clob
    }

    pub fn submit(&mut self, command: Command) -> Vec<Event> {
        let mut out = Vec::new();
        self.submit_into(command, &mut out);
        out
    }

    pub fn submit_into(&mut self, command: Command, out: &mut Vec<Event>) {
        let seq = self.sequencer.next_seq();

        let result = if self.gateway.has_risk() {
            let ctx = self.risk_context(&command);
            self.gateway.validate_risk(&command, &ctx)
        } else {
            self.gateway.validate(&command)
        };
        if let Err(reason) = result {
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

    fn risk_context(&self, command: &Command) -> RiskContext {
        let book = self.engine.book();
        let mid = book.mid();
        match command {
            Command::New(order) => RiskContext {
                mid,
                owner: order.owner,
                side: order.side,
                net: book.account_net(order.owner),
                open: book.account_open(order.owner, order.side),
            },
            Command::Modify(modify) => match book.get(modify.order_id) {
                Some((side, current)) => {
                    let own = current.qty
                        + book
                            .reserve(modify.order_id)
                            .map_or(0, |(_, hidden)| hidden);
                    let open = book.account_open(current.owner, side).saturating_sub(own);
                    RiskContext {
                        mid,
                        owner: current.owner,
                        side,
                        net: book.account_net(current.owner),
                        open,
                    }
                }
                None => RiskContext::INERT,
            },
            Command::Cancel(_) => RiskContext::INERT,
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

    pub(crate) fn set_risk(&mut self, risk: RiskConfig) {
        self.gateway = Gateway::with_config(risk);
    }

    pub(crate) fn set_fees(&mut self, fees: FeeConfig) {
        self.engine.set_fees(fees);
    }

    pub(crate) fn capture(&self) -> SnapshotState {
        let (seq, next_order_id) = self.sequencer.snapshot();
        SnapshotState {
            seq,
            next_order_id,
            last_trade_price: self.engine.last_trade_price(),
            orders: self.engine.resting_orders(),
            stops: self.engine.pending_stops_data(),
            account_net: self.engine.book().account_nets(),
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
        for (owner, net) in state.account_net {
            engine.restore_account_net(owner, net);
        }
        Clob {
            gateway: Gateway::new(),
            sequencer: Sequencer::restore(state.seq, state.next_order_id),
            engine,
        }
    }
}
