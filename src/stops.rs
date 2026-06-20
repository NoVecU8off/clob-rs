use crate::types::{OrderId, OrderType, Price, Qty, Side, TimeInForce};

#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingStop {
    pub(crate) id: OrderId,
    pub(crate) side: Side,
    pub(crate) trigger: Price,
    pub(crate) activates_to: OrderType,
    pub(crate) limit_price: Price,
    pub(crate) qty: Qty,
    pub(crate) tif: TimeInForce,
}

impl PendingStop {
    fn triggered_by(&self, last: Price) -> bool {
        match self.side {
            Side::Buy => last >= self.trigger,
            Side::Sell => last <= self.trigger,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct StopBook {
    pending: Vec<PendingStop>,
}

impl StopBook {
    pub(crate) fn len(&self) -> usize {
        self.pending.len()
    }

    pub(crate) fn pending(&self) -> &[PendingStop] {
        &self.pending
    }

    pub(crate) fn park(&mut self, stop: PendingStop) {
        self.pending.push(stop);
    }

    pub(crate) fn cancel(&mut self, order_id: OrderId) -> Option<PendingStop> {
        let pos = self.pending.iter().position(|s| s.id == order_id)?;
        Some(self.pending.remove(pos))
    }

    pub(crate) fn take_triggered(&mut self, last: Price) -> Option<PendingStop> {
        let pos = self.pending.iter().position(|s| s.triggered_by(last))?;
        Some(self.pending.remove(pos))
    }
}
