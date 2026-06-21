use crate::types::{Price, Qty, Side};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RiskConfig {
    pub tick_size: Price,
    pub lot_size: Qty,
    pub band_ticks: u64,
    pub position_limit: Qty,
}

impl RiskConfig {
    pub fn new() -> Self {
        RiskConfig::default()
    }

    pub fn with_tick(mut self, tick_size: Price) -> Self {
        self.tick_size = tick_size;
        self
    }

    pub fn with_lot(mut self, lot_size: Qty) -> Self {
        self.lot_size = lot_size;
        self
    }

    pub fn with_price_band(mut self, band_ticks: u64) -> Self {
        self.band_ticks = band_ticks;
        self
    }

    pub fn with_position_limit(mut self, position_limit: Qty) -> Self {
        self.position_limit = position_limit;
        self
    }

    pub(crate) fn is_active(&self) -> bool {
        self.tick_size > 1 || self.lot_size > 1 || self.band_ticks > 0 || self.position_limit > 0
    }

    pub(crate) fn tick_ok(&self, price: Price) -> bool {
        self.tick_size <= 1 || price.is_multiple_of(self.tick_size)
    }

    pub(crate) fn lot_ok(&self, qty: Qty) -> bool {
        self.lot_size <= 1 || qty.is_multiple_of(self.lot_size)
    }

    pub(crate) fn band_ok(&self, mid: Option<Price>, price: Price) -> bool {
        if self.band_ticks == 0 {
            return true;
        }
        let Some(mid) = mid else {
            return true;
        };
        let offset = (self.band_ticks as u128) * (self.tick_size.max(1) as u128);
        let lo = (mid as u128).saturating_sub(offset);
        let hi = (mid as u128).saturating_add(offset);
        let price = price as u128;
        price >= lo && price <= hi
    }

    pub(crate) fn position_ok(&self, net: i128, open: Qty, side: Side, qty: Qty) -> bool {
        if self.position_limit == 0 {
            return true;
        }
        let projected = match side {
            Side::Buy => net + open as i128 + qty as i128,
            Side::Sell => -net + open as i128 + qty as i128,
        };
        projected <= self.position_limit as i128
    }
}
