#![allow(dead_code)]

use std::path::{Path, PathBuf};

use clob::{
    CancelOrder, Clob, Command, Event, ModifyOrder, NewOrder, OrderBook, Side, TimeInForce,
};

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

pub struct TempFile(PathBuf);

impl TempFile {
    pub fn new(name: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!("clob_persist_{}_{}.wal", std::process::id(), name));
        let _ = std::fs::remove_file(&path);
        TempFile(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        for suffix in [".snap", ".tmp", ".snap.tmp"] {
            let mut sidecar = self.0.clone().into_os_string();
            sidecar.push(suffix);
            let _ = std::fs::remove_file(PathBuf::from(sidecar));
        }
    }
}

pub fn snap_of(path: &Path) -> PathBuf {
    let mut name = path.to_path_buf().into_os_string();
    name.push(".snap");
    PathBuf::from(name)
}

pub type State = (
    Option<u64>,
    Option<u64>,
    Vec<(u64, u64)>,
    Vec<(u64, u64)>,
    usize,
    usize,
    u64,
);

pub fn capture(book: &OrderBook, pending: usize, seq: u64) -> State {
    (
        book.best_bid(),
        book.best_ask(),
        book.depth(Side::Buy, 64),
        book.depth(Side::Sell, 64),
        book.len(),
        pending,
        seq,
    )
}

pub fn script() -> Vec<Command> {
    vec![
        Command::New(NewOrder::limit(Side::Sell, 101, 10)),
        Command::New(NewOrder::limit(Side::Sell, 102, 5)),
        Command::New(NewOrder::limit(Side::Buy, 100, 7)),
        Command::New(NewOrder::limit(Side::Buy, 101, 12)),
        Command::New(NewOrder::limit(Side::Buy, 100, 0)),
        Command::Cancel(CancelOrder { order_id: 2 }),
        Command::New(NewOrder::iceberg(Side::Sell, 105, 20, 5)),
        Command::New(NewOrder::limit(Side::Buy, 105, 7)),
        Command::New(NewOrder::stop(Side::Buy, 200, 5)),
        Command::Modify(ModifyOrder::new(4, 99, 2)),
        Command::New(NewOrder::limit(Side::Sell, 100, 1).with_tif(TimeInForce::Ioc)),
    ]
}
