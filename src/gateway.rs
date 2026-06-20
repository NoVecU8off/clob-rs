use crate::error::RejectReason;
use crate::order::{Command, ModifyOrder, NewOrder};
use crate::types::OrderType;

#[derive(Clone, Copy, Debug, Default)]
pub struct Gateway;

impl Gateway {
    pub fn new() -> Self {
        Gateway
    }

    pub fn validate(&self, command: &Command) -> Result<(), RejectReason> {
        match command {
            Command::New(order) => self.validate_new(order),
            Command::Cancel(_) => Ok(()),
            Command::Modify(modify) => self.validate_modify(modify),
        }
    }

    fn validate_new(&self, order: &NewOrder) -> Result<(), RejectReason> {
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
        }
        Ok(())
    }

    fn validate_modify(&self, modify: &ModifyOrder) -> Result<(), RejectReason> {
        if modify.qty == 0 {
            return Err(RejectReason::ZeroQuantity);
        }
        if modify.price == 0 {
            return Err(RejectReason::InvalidPrice);
        }
        Ok(())
    }
}
