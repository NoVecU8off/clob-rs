#![allow(dead_code)]

use clob::{Clob, Command, Event, NewOrder, Side};

pub fn trades(events: &[Event]) -> Vec<(u64, u64, u64)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Trade {
                maker_order_id,
                price,
                qty,
                ..
            } => Some((*maker_order_id, *price, *qty)),
            _ => None,
        })
        .collect()
}

pub fn resting(events: &[Event]) -> Option<(u64, u64)> {
    events.iter().find_map(|e| match e {
        Event::Resting { price, qty, .. } => Some((*price, *qty)),
        _ => None,
    })
}

pub fn accepted_id(events: &[Event]) -> u64 {
    events
        .iter()
        .find_map(|e| match e {
            Event::Accepted { order_id, .. } => Some(*order_id),
            _ => None,
        })
        .expect("expected an Accepted event")
}

pub fn place_limit(clob: &mut Clob, side: Side, price: u64, qty: u64) -> u64 {
    let events = clob.submit(Command::New(NewOrder::limit(side, price, qty)));
    accepted_id(&events)
}
