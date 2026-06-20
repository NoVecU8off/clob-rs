mod common;

use std::collections::BTreeMap;

use clob::{
    CancelOrder, Clob, Command, L2Feed, L2Level, L2Update, ModifyOrder, NewOrder, Side, TimeInForce,
};
use common::{accepted_id, script};

const ALL: usize = usize::MAX;

#[derive(Default, PartialEq, Eq, Debug)]
struct Mirror {
    bids: BTreeMap<u64, u64>,
    asks: BTreeMap<u64, u64>,
}

impl Mirror {
    fn apply(&mut self, update: &L2Update) {
        for level in &update.bids {
            put(&mut self.bids, level);
        }
        for level in &update.asks {
            put(&mut self.asks, level);
        }
    }
}

fn put(side: &mut BTreeMap<u64, u64>, level: &L2Level) {
    if level.qty == 0 {
        side.remove(&level.price);
    } else {
        side.insert(level.price, level.qty);
    }
}

fn live(clob: &Clob) -> Mirror {
    let l2 = clob.l2_snapshot(ALL);
    Mirror {
        bids: l2.bids.iter().map(|l| (l.price, l.qty)).collect(),
        asks: l2.asks.iter().map(|l| (l.price, l.qty)).collect(),
    }
}

fn step(clob: &mut Clob, feed: &mut L2Feed, command: Command) -> L2Update {
    let events = clob.submit(command);
    feed.apply(&events, clob.book())
}

#[test]
fn add_levels_aggregate_per_price() {
    let mut clob = Clob::new();
    let mut feed = L2Feed::new();

    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Sell, 101, 10)),
    );
    assert_eq!(
        u.asks,
        vec![L2Level {
            price: 101,
            qty: 10
        }]
    );
    assert!(u.bids.is_empty());

    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Sell, 101, 5)),
    );
    assert_eq!(
        u.asks,
        vec![L2Level {
            price: 101,
            qty: 15
        }]
    );
}

#[test]
fn trade_reduces_then_removes_maker_level() {
    let mut clob = Clob::new();
    let mut feed = L2Feed::new();
    step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Sell, 101, 10)),
    );

    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Buy, 101, 4)),
    );
    assert_eq!(u.asks, vec![L2Level { price: 101, qty: 6 }]);

    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Buy, 101, 6)),
    );
    assert_eq!(u.asks, vec![L2Level { price: 101, qty: 0 }]);
}

#[test]
fn cancel_removes_level() {
    let mut clob = Clob::new();
    let mut feed = L2Feed::new();
    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 5)));
    let id = accepted_id(&events);
    feed.apply(&events, clob.book());

    let u = step(
        &mut clob,
        &mut feed,
        Command::Cancel(CancelOrder { order_id: id }),
    );
    assert_eq!(u.bids, vec![L2Level { price: 100, qty: 0 }]);
    assert!(u.asks.is_empty());
}

#[test]
fn modify_reprice_moves_qty_best_first() {
    let mut clob = Clob::new();
    let mut feed = L2Feed::new();
    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 5)));
    let id = accepted_id(&events);
    feed.apply(&events, clob.book());

    let u = step(
        &mut clob,
        &mut feed,
        Command::Modify(ModifyOrder::new(id, 99, 5)),
    );
    assert_eq!(
        u.bids,
        vec![
            L2Level { price: 100, qty: 0 },
            L2Level { price: 99, qty: 5 },
        ]
    );
}

#[test]
fn modify_reduce_in_place_keeps_level() {
    let mut clob = Clob::new();
    let mut feed = L2Feed::new();
    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 10)));
    let id = accepted_id(&events);
    feed.apply(&events, clob.book());

    let u = step(
        &mut clob,
        &mut feed,
        Command::Modify(ModifyOrder::new(id, 100, 4)),
    );
    assert_eq!(u.bids, vec![L2Level { price: 100, qty: 4 }]);
}

#[test]
fn sweep_two_levels_emits_sorted_asks() {
    let mut clob = Clob::new();
    let mut feed = L2Feed::new();
    step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Sell, 102, 5)),
    );
    step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Sell, 101, 5)),
    );

    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::market(Side::Buy, 7)),
    );
    assert_eq!(
        u.asks,
        vec![
            L2Level { price: 101, qty: 0 },
            L2Level { price: 102, qty: 3 },
        ]
    );
}

#[test]
fn iceberg_refill_tracks_visible_only() {
    let mut clob = Clob::new();
    let mut feed = L2Feed::new();

    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::iceberg(Side::Sell, 105, 20, 5)),
    );
    assert_eq!(u.asks, vec![L2Level { price: 105, qty: 5 }]);

    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Buy, 105, 7)),
    );
    assert_eq!(u.asks, vec![L2Level { price: 105, qty: 3 }]);
    assert_eq!(live(&clob), {
        let mut m = Mirror::default();
        m.asks.insert(105, 3);
        m
    });
}

#[test]
fn stop_cascade_reflects_final_book() {
    let mut clob = Clob::new();
    let mut feed = L2Feed::new();
    step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Sell, 101, 10)),
    );
    step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::stop(Side::Buy, 101, 5)),
    );

    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Buy, 101, 1)),
    );
    assert_eq!(u.asks, vec![L2Level { price: 101, qty: 4 }]);
    assert!(u.bids.is_empty());
}

#[test]
fn reject_advances_seq_with_no_changes() {
    let mut clob = Clob::new();
    let mut feed = L2Feed::new();
    step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Sell, 101, 5)),
    );

    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Buy, 100, 0)),
    );
    assert_eq!(u.seq, 2);
    assert!(u.bids.is_empty() && u.asks.is_empty());
}

#[test]
fn seeded_from_book_stays_in_sync() {
    let mut clob = Clob::new();
    let a = accepted_id(&clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 5))));
    clob.submit(Command::New(NewOrder::limit(Side::Buy, 99, 8)));
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 105, 4)));

    let mut feed = L2Feed::from_book(clob.book());
    let mut mirror = live(&clob);

    for command in [
        Command::Cancel(CancelOrder { order_id: a }),
        Command::New(NewOrder::limit(Side::Sell, 99, 3)),
        Command::New(NewOrder::limit(Side::Buy, 105, 2)),
    ] {
        let u = step(&mut clob, &mut feed, command);
        mirror.apply(&u);
        assert_eq!(mirror, live(&clob));
    }
}

#[test]
fn script_replays_to_live_l2_each_step() {
    let mut clob = Clob::new();
    let mut feed = L2Feed::new();
    let mut mirror = Mirror::default();

    for command in script() {
        let events = clob.submit(command);
        let u = feed.apply(&events, clob.book());
        assert_eq!(u.seq, clob.current_seq());
        mirror.apply(&u);
        assert_eq!(
            mirror,
            live(&clob),
            "L2 mirror desynced at seq {}",
            clob.current_seq()
        );
    }
}

#[test]
fn ioc_remainder_cancel_does_not_touch_book() {
    let mut clob = Clob::new();
    let mut feed = L2Feed::new();
    step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Sell, 101, 4)),
    );

    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Buy, 101, 10).with_tif(TimeInForce::Ioc)),
    );
    assert_eq!(u.asks, vec![L2Level { price: 101, qty: 0 }]);
    assert!(u.bids.is_empty());
}
