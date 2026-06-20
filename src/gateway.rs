use crate::error::RejectReason;
use crate::order::{Command, NewOrder};
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
        }
    }

    fn validate_new(&self, order: &NewOrder) -> Result<(), RejectReason> {
        if order.qty == 0 {
            return Err(RejectReason::ZeroQuantity);
        }
        if order.order_type == OrderType::Limit && order.price == 0 {
            return Err(RejectReason::InvalidPrice);
        }
        Ok(())
    }
}
