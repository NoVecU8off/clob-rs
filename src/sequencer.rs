use crate::types::{OrderId, SeqNum};

#[derive(Clone, Copy, Debug)]
pub struct Sequencer {
    seq: SeqNum,
    next_order_id: OrderId,
}

impl Sequencer {
    pub fn new() -> Self {
        Sequencer {
            seq: 0,
            next_order_id: 1,
        }
    }

    pub fn next_seq(&mut self) -> SeqNum {
        self.seq += 1;
        self.seq
    }

    pub fn next_order_id(&mut self) -> OrderId {
        let id = self.next_order_id;
        self.next_order_id += 1;
        id
    }

    pub fn current_seq(&self) -> SeqNum {
        self.seq
    }

    pub(crate) fn snapshot(&self) -> (SeqNum, OrderId) {
        (self.seq, self.next_order_id)
    }

    pub(crate) fn restore(seq: SeqNum, next_order_id: OrderId) -> Self {
        Sequencer { seq, next_order_id }
    }
}

impl Default for Sequencer {
    fn default() -> Self {
        Sequencer::new()
    }
}
