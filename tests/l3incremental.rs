mod common;

use std::collections::{BTreeMap, HashMap};

use clob::{
    CancelOrder, Clob, Command, L3Delta, L3Feed, L3Update, ModifyOrder, NewOrder, Side, TimeInForce,
};
use common::{accepted_id, script};

type Book = Vec<(u64, u64, u64)>;

#[derive(Default, PartialEq, Eq, Debug)]
struct Mirror {
    bids: BTreeMap<u64, Vec<(u64, u64)>>,
    asks: BTreeMap<u64, Vec<(u64, u64)>>,
    loc: HashMap<u64, (Side, u64)>,
}

impl Mirror {
    fn apply(&mut self, update: &L3Update) {
        for delta in &update.deltas {
            match *delta {
                L3Delta::Added {
                    id,
                    side,
                    price,
                    qty,
                } => {
                    self.loc.insert(id, (side, price));
                    self.side(side).entry(price).or_default().push((id, qty));
                }
                L3Delta::Reduced { id, qty } => {
                    let (side, price) = self.loc[&id];
                    for slot in self.side(side).get_mut(&price).unwrap() {
                        if slot.0 == id {
                            slot.1 = qty;
                        }
                    }
                }
                L3Delta::Removed { id } => {
                    let (side, price) = self.loc.remove(&id).unwrap();
                    let level = self.side(side).get_mut(&price).unwrap();
                    level.retain(|&(oid, _)| oid != id);
                    if level.is_empty() {
                        self.side(side).remove(&price);
                    }
                }
            }
        }
    }

    fn side(&mut self, side: Side) -> &mut BTreeMap<u64, Vec<(u64, u64)>> {
        match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        }
    }
}

fn flat(book: &BTreeMap<u64, Vec<(u64, u64)>>, best_first_desc: bool) -> Book {
    let mut out = Vec::new();
    let prices: Vec<u64> = if best_first_desc {
        book.keys().rev().copied().collect()
    } else {
        book.keys().copied().collect()
    };
    for price in prices {
        for &(id, qty) in &book[&price] {
            out.push((id, price, qty));
        }
    }
    out
}

fn mirror_flat(mirror: &Mirror) -> (Book, Book) {
    (flat(&mirror.bids, true), flat(&mirror.asks, false))
}

fn live(clob: &Clob) -> (Book, Book) {
    let l3 = clob.l3_snapshot();
    (
        l3.bids.iter().map(|o| (o.id, o.price, o.qty)).collect(),
        l3.asks.iter().map(|o| (o.id, o.price, o.qty)).collect(),
    )
}

fn step(clob: &mut Clob, feed: &mut L3Feed, command: Command) -> L3Update {
    let events = clob.submit(command);
    feed.apply(&events, clob.book())
}

#[test]
fn rest_emits_add_at_tail() {
    let mut clob = Clob::new();
    let mut feed = L3Feed::new();

    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Sell, 101, 10)),
    );
    assert_eq!(
        u.deltas,
        vec![L3Delta::Added {
            id: 1,
            side: Side::Sell,
            price: 101,
            qty: 10
        }]
    );

    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Sell, 101, 5)),
    );
    assert_eq!(
        u.deltas,
        vec![L3Delta::Added {
            id: 2,
            side: Side::Sell,
            price: 101,
            qty: 5
        }]
    );
}

#[test]
fn trade_reduces_then_removes_maker() {
    let mut clob = Clob::new();
    let mut feed = L3Feed::new();
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
    assert_eq!(u.deltas, vec![L3Delta::Reduced { id: 1, qty: 6 }]);

    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Buy, 101, 6)),
    );
    assert_eq!(u.deltas, vec![L3Delta::Removed { id: 1 }]);
}

#[test]
fn cancel_removes_order() {
    let mut clob = Clob::new();
    let mut feed = L3Feed::new();
    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 5)));
    let id = accepted_id(&events);
    feed.apply(&events, clob.book());

    let u = step(
        &mut clob,
        &mut feed,
        Command::Cancel(CancelOrder { order_id: id }),
    );
    assert_eq!(u.deltas, vec![L3Delta::Removed { id }]);
}

#[test]
fn iceberg_refill_requeues_behind_resting_order() {
    let mut clob = Clob::new();
    let mut feed = L3Feed::new();
    let mut mirror = Mirror::default();

    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::iceberg(Side::Sell, 105, 20, 5)),
    );
    mirror.apply(&u);
    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Sell, 105, 4)),
    );
    mirror.apply(&u);

    let u = step(
        &mut clob,
        &mut feed,
        Command::New(NewOrder::limit(Side::Buy, 105, 7)),
    );
    assert_eq!(
        u.deltas,
        vec![
            L3Delta::Removed { id: 1 },
            L3Delta::Reduced { id: 2, qty: 2 },
            L3Delta::Added {
                id: 1,
                side: Side::Sell,
                price: 105,
                qty: 5
            },
        ]
    );
    mirror.apply(&u);

    let (_, asks) = mirror_flat(&mirror);
    assert_eq!(asks, vec![(2, 105, 2), (1, 105, 5)]);
    assert_eq!(mirror_flat(&mirror), live(&clob));
}

#[test]
fn modify_reprice_moves_order_across_levels() {
    let mut clob = Clob::new();
    let mut feed = L3Feed::new();
    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 5)));
    let id = accepted_id(&events);
    feed.apply(&events, clob.book());

    let u = step(
        &mut clob,
        &mut feed,
        Command::Modify(ModifyOrder::new(id, 99, 5)),
    );
    assert_eq!(
        u.deltas,
        vec![
            L3Delta::Removed { id },
            L3Delta::Added {
                id,
                side: Side::Buy,
                price: 99,
                qty: 5
            },
        ]
    );
}

#[test]
fn modify_increase_loses_priority_within_level() {
    let mut clob = Clob::new();
    let a = accepted_id(&clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 5))));
    let b = accepted_id(&clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 3))));
    let mut feed = L3Feed::from_book(clob.book());

    let u = step(
        &mut clob,
        &mut feed,
        Command::Modify(ModifyOrder::new(a, 100, 8)),
    );
    assert_eq!(
        u.deltas,
        vec![
            L3Delta::Removed { id: a },
            L3Delta::Added {
                id: a,
                side: Side::Buy,
                price: 100,
                qty: 8
            },
        ]
    );
    assert_eq!(b, 2);
}

#[test]
fn modify_reduce_keeps_position() {
    let mut clob = Clob::new();
    let id = accepted_id(&clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 10))));
    let mut feed = L3Feed::from_book(clob.book());

    let u = step(
        &mut clob,
        &mut feed,
        Command::Modify(ModifyOrder::new(id, 100, 4)),
    );
    assert_eq!(u.deltas, vec![L3Delta::Reduced { id, qty: 4 }]);
}

#[test]
fn reject_advances_seq_with_no_deltas() {
    let mut clob = Clob::new();
    let mut feed = L3Feed::new();
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
    assert!(u.deltas.is_empty());
}

#[test]
fn ioc_remainder_cancel_does_not_touch_book() {
    let mut clob = Clob::new();
    let mut feed = L3Feed::new();
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
    assert_eq!(u.deltas, vec![L3Delta::Removed { id: 1 }]);
}

#[test]
fn seeded_from_book_stays_in_sync() {
    let mut clob = Clob::new();
    let a = accepted_id(&clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 5))));
    clob.submit(Command::New(NewOrder::limit(Side::Buy, 99, 8)));
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 105, 4)));

    let mut feed = L3Feed::from_book(clob.book());
    let mut mirror = Mirror::default();
    let l3 = clob.l3_snapshot();
    for o in l3.bids.iter().chain(l3.asks.iter()) {
        mirror.apply(&L3Update {
            seq: l3.seq,
            deltas: vec![L3Delta::Added {
                id: o.id,
                side: o.side,
                price: o.price,
                qty: o.qty,
            }],
        });
    }
    assert_eq!(mirror_flat(&mirror), live(&clob));

    for command in [
        Command::Cancel(CancelOrder { order_id: a }),
        Command::New(NewOrder::limit(Side::Sell, 99, 3)),
        Command::New(NewOrder::limit(Side::Buy, 105, 2)),
    ] {
        let u = step(&mut clob, &mut feed, command);
        mirror.apply(&u);
        assert_eq!(mirror_flat(&mirror), live(&clob));
    }
}

#[test]
fn script_replays_to_live_l3_each_step() {
    let mut clob = Clob::new();
    let mut feed = L3Feed::new();
    let mut mirror = Mirror::default();

    for command in script() {
        let events = clob.submit(command);
        let u = feed.apply(&events, clob.book());
        assert_eq!(u.seq, clob.current_seq());
        mirror.apply(&u);
        assert_eq!(
            mirror_flat(&mirror),
            live(&clob),
            "L3 mirror desynced at seq {}",
            clob.current_seq()
        );
    }
}
