use crate::error::RejectReason;
use crate::order::{Command, ModifyOrder, NewOrder};
use crate::risk::RiskConfig;
use crate::types::{AccountId, OrderType, Price, Qty, Side};

#[derive(Clone, Copy, Debug, Default)]
pub struct Gateway {
    risk: RiskConfig,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RiskContext {
    pub(crate) mid: Option<Price>,
    pub(crate) owner: AccountId,
    pub(crate) side: Side,
    pub(crate) net: i128,
    pub(crate) open: Qty,
}

impl RiskContext {
    pub(crate) const INERT: RiskContext = RiskContext {
        mid: None,
        owner: 0,
        side: Side::Buy,
        net: 0,
        open: 0,
    };
}

impl Gateway {
    pub fn new() -> Self {
        Gateway::default()
    }

    pub fn with_config(risk: RiskConfig) -> Self {
        Gateway { risk }
    }

    pub fn validate(&self, command: &Command) -> Result<(), RejectReason> {
        structural(command)
    }

    pub(crate) fn has_risk(&self) -> bool {
        self.risk.is_active()
    }

    pub(crate) fn validate_risk(
        &self,
        command: &Command,
        ctx: &RiskContext,
    ) -> Result<(), RejectReason> {
        structural(command)?;
        if !self.risk.is_active() {
            return Ok(());
        }
        match command {
            Command::New(order) => self.risk_new(order, ctx),
            Command::Cancel(_) => Ok(()),
            Command::Modify(modify) => self.risk_modify(modify, ctx),
        }
    }

    fn risk_new(&self, order: &NewOrder, ctx: &RiskContext) -> Result<(), RejectReason> {
        if !self.risk.lot_ok(order.qty) {
            return Err(RejectReason::LotSize);
        }
        if let OrderType::Iceberg { display } = order.order_type
            && !self.risk.lot_ok(display)
        {
            return Err(RejectReason::LotSize);
        }
        for price in order_prices(order).into_iter().flatten() {
            if !self.risk.tick_ok(price) {
                return Err(RejectReason::TickSize);
            }
        }
        if let Some(price) = corridor_price(order)
            && !self.risk.band_ok(ctx.mid, price)
        {
            return Err(RejectReason::PriceBand);
        }
        if order.owner != 0
            && !self
                .risk
                .position_ok(ctx.net, ctx.open, order.side, order.qty)
        {
            return Err(RejectReason::PositionLimit);
        }
        Ok(())
    }

    fn risk_modify(&self, modify: &ModifyOrder, ctx: &RiskContext) -> Result<(), RejectReason> {
        if !self.risk.lot_ok(modify.qty) {
            return Err(RejectReason::LotSize);
        }
        if !self.risk.tick_ok(modify.price) {
            return Err(RejectReason::TickSize);
        }
        if !self.risk.band_ok(ctx.mid, modify.price) {
            return Err(RejectReason::PriceBand);
        }
        if ctx.owner != 0
            && !self
                .risk
                .position_ok(ctx.net, ctx.open, ctx.side, modify.qty)
        {
            return Err(RejectReason::PositionLimit);
        }
        Ok(())
    }
}

fn structural(command: &Command) -> Result<(), RejectReason> {
    match command {
        Command::New(order) => structural_new(order),
        Command::Cancel(_) => Ok(()),
        Command::Modify(modify) => structural_modify(modify),
    }
}

fn structural_new(order: &NewOrder) -> Result<(), RejectReason> {
    if order.qty == 0 {
        return Err(RejectReason::ZeroQuantity);
    }
    match order.order_type {
        OrderType::Limit => {
            if order.price == 0 {
                return Err(RejectReason::InvalidPrice);
            }
        }
        OrderType::Market => {}
        OrderType::Stop { trigger } => {
            if trigger == 0 {
                return Err(RejectReason::InvalidPrice);
            }
        }
        OrderType::StopLimit { trigger } => {
            if trigger == 0 || order.price == 0 {
                return Err(RejectReason::InvalidPrice);
            }
        }
        OrderType::Iceberg { display } => {
            if order.price == 0 {
                return Err(RejectReason::InvalidPrice);
            }
            if display == 0 {
                return Err(RejectReason::ZeroQuantity);
            }
        }
    }
    Ok(())
}

fn structural_modify(modify: &ModifyOrder) -> Result<(), RejectReason> {
    if modify.qty == 0 {
        return Err(RejectReason::ZeroQuantity);
    }
    if modify.price == 0 {
        return Err(RejectReason::InvalidPrice);
    }
    Ok(())
}

fn order_prices(order: &NewOrder) -> [Option<Price>; 2] {
    match order.order_type {
        OrderType::Limit | OrderType::Iceberg { .. } => [Some(order.price), None],
        OrderType::Market => [None, None],
        OrderType::Stop { trigger } => [Some(trigger), None],
        OrderType::StopLimit { trigger } => [Some(trigger), Some(order.price)],
    }
}

fn corridor_price(order: &NewOrder) -> Option<Price> {
    match order.order_type {
        OrderType::Limit | OrderType::Iceberg { .. } => Some(order.price),
        _ => None,
    }
}
