mod common;

use std::collections::BTreeMap;

use clob::{Clob, Command, L2Level, L3Order, NewOrder, Side};
use common::{accepted_id, place_limit};

#[test]
fn l2_aggregates_per_level_best_first() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Sell, 101, 10);
    place_limit(&mut clob, Side::Sell, 101, 5);
    place_limit(&mut clob, Side::Sell, 102, 5);
    place_limit(&mut clob, Side::Buy, 100, 7);
    place_limit(&mut clob, Side::Buy, 100, 3);

    let l2 = clob.l2_snapshot(10);
    assert_eq!(
        l2.asks,
        vec![
            L2Level {
                price: 101,
                qty: 15
            },
            L2Level { price: 102, qty: 5 },
        ]
    );
    assert_eq!(
        l2.bids,
        vec![L2Level {
            price: 100,
            qty: 10
        }]
    );
}

#[test]
fn l2_respects_depth_limit() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Sell, 101, 1);
    place_limit(&mut clob, Side::Sell, 102, 1);
    place_limit(&mut clob, Side::Sell, 103, 1);

    let l2 = clob.l2_snapshot(2);
    assert_eq!(l2.asks.len(), 2);
    assert_eq!(l2.asks[0].price, 101);
    assert_eq!(l2.asks[1].price, 102);
}

#[test]
fn snapshot_seq_tracks_current_seq_including_rejects() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Sell, 101, 5);
    clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 0)));
    place_limit(&mut clob, Side::Buy, 100, 4);

    assert_eq!(clob.current_seq(), 3);
    assert_eq!(clob.l2_snapshot(10).seq, 3);
    assert_eq!(clob.l3_snapshot().seq, 3);
}

#[test]
fn l3_orders_best_price_then_fifo() {
    let mut clob = Clob::new();
    let a = place_limit(&mut clob, Side::Buy, 100, 10);
    let b = place_limit(&mut clob, Side::Buy, 101, 5);
    let c = place_limit(&mut clob, Side::Buy, 100, 7);
    let d = place_limit(&mut clob, Side::Sell, 105, 3);

    let l3 = clob.l3_snapshot();
    assert_eq!(
        l3.bids,
        vec![
            L3Order {
                id: b,
                side: Side::Buy,
                price: 101,
                qty: 5
            },
            L3Order {
                id: a,
                side: Side::Buy,
                price: 100,
                qty: 10
            },
            L3Order {
                id: c,
                side: Side::Buy,
                price: 100,
                qty: 7
            },
        ]
    );
    assert_eq!(
        l3.asks,
        vec![L3Order {
            id: d,
            side: Side::Sell,
            price: 105,
            qty: 3
        }]
    );
}

#[test]
fn l3_hides_iceberg_reserve() {
    let mut clob = Clob::new();
    let events = clob.submit(Command::New(NewOrder::iceberg(Side::Sell, 105, 20, 5)));
    let id = accepted_id(&events);

    let l3 = clob.l3_snapshot();
    assert_eq!(l3.asks.len(), 1);
    assert_eq!(l3.asks[0].id, id);
    assert_eq!(l3.asks[0].qty, 5);

    let l2 = clob.l2_snapshot(10);
    assert_eq!(l2.asks, vec![L2Level { price: 105, qty: 5 }]);
}

#[test]
fn l2_equals_l3_aggregated() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Buy, 100, 4);
    place_limit(&mut clob, Side::Buy, 100, 6);
    place_limit(&mut clob, Side::Buy, 99, 8);
    place_limit(&mut clob, Side::Sell, 101, 3);
    place_limit(&mut clob, Side::Sell, 102, 5);
    clob.submit(Command::New(NewOrder::iceberg(Side::Sell, 101, 30, 7)));

    let l2 = clob.l2_snapshot(64);
    let l3 = clob.l3_snapshot();

    assert_eq!(level_map(&l2.bids), aggregate(&l3.bids));
    assert_eq!(level_map(&l2.asks), aggregate(&l3.asks));
}

#[test]
fn snapshot_reflects_post_trade_book() {
    let mut clob = Clob::new();
    let maker = place_limit(&mut clob, Side::Sell, 101, 10);
    place_limit(&mut clob, Side::Buy, 101, 4);

    let l2 = clob.l2_snapshot(10);
    assert_eq!(l2.asks, vec![L2Level { price: 101, qty: 6 }]);
    assert!(l2.bids.is_empty());

    let l3 = clob.l3_snapshot();
    assert_eq!(
        l3.asks,
        vec![L3Order {
            id: maker,
            side: Side::Sell,
            price: 101,
            qty: 6
        }]
    );
}

#[test]
fn empty_book_has_empty_snapshots() {
    let clob = Clob::new();
    let l2 = clob.l2_snapshot(10);
    let l3 = clob.l3_snapshot();
    assert_eq!(l2.seq, 0);
    assert!(l2.bids.is_empty() && l2.asks.is_empty());
    assert!(l3.bids.is_empty() && l3.asks.is_empty());
}

fn level_map(levels: &[L2Level]) -> BTreeMap<u64, u64> {
    levels.iter().map(|l| (l.price, l.qty)).collect()
}

fn aggregate(orders: &[L3Order]) -> BTreeMap<u64, u64> {
    let mut m = BTreeMap::new();
    for o in orders {
        *m.entry(o.price).or_insert(0) += o.qty;
    }
    m
}
