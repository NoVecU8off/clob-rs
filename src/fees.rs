use crate::types::{Price, Qty};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FeeConfig {
    pub maker_ppm: i64,
    pub taker_ppm: i64,
}

impl FeeConfig {
    pub fn new() -> Self {
        FeeConfig::default()
    }

    pub fn with_maker_ppm(mut self, maker_ppm: i64) -> Self {
        self.maker_ppm = maker_ppm;
        self
    }

    pub fn with_taker_ppm(mut self, taker_ppm: i64) -> Self {
        self.taker_ppm = taker_ppm;
        self
    }

    pub(crate) fn taker_fee(&self, price: Price, qty: Qty) -> i64 {
        Self::charge(self.taker_ppm, price, qty)
    }

    pub(crate) fn maker_fee(&self, price: Price, qty: Qty) -> i64 {
        Self::charge(self.maker_ppm, price, qty)
    }

    fn charge(rate_ppm: i64, price: Price, qty: Qty) -> i64 {
        if rate_ppm == 0 {
            return 0;
        }
        let notional = (price as i128).saturating_mul(qty as i128);
        let scaled = notional.saturating_mul(rate_ppm as i128);
        let fee = scaled / 1_000_000;
        fee.clamp(i64::MIN as i128, i64::MAX as i128) as i64
    }
}
