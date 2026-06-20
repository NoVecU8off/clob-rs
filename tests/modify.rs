mod common;

use clob::{Clob, Command, Event, ModifyOrder, NewOrder, RejectReason, Side};

use common::{place_limit, resting, trades};

fn modify(clob: &mut Clob, order_id: u64, price: u64, qty: u64) -> Vec<Event> {
    clob.submit(Command::Modify(ModifyOrder::new(order_id, price, qty)))
}

fn has_modified(events: &[Event], id: u64) -> bool {
    events
        .iter()
        .any(|e| matches!(e, Event::Modified { order_id, .. } if *order_id == id))
}

#[test]
fn reduce_qty_same_price_keeps_priority() {
    let mut clob = Clob::new();
    let a = place_limit(&mut clob, Side::Buy, 100, 10);
    let b = place_limit(&mut clob, Side::Buy, 100, 5);

    let ev = modify(&mut clob, a, 100, 6);
    assert!(has_modified(&ev, a));
    assert!(trades(&ev).is_empty());
    assert_eq!(resting(&ev), Some((100, 6)));

    let sell = clob.submit(Command::New(NewOrder::limit(Side::Sell, 100, 6)));
    assert_eq!(trades(&sell), vec![(a, 100, 6)]);
    assert!(!clob.book().contains(a));
    assert!(clob.book().contains(b));
}

#[test]
fn increase_qty_loses_priority() {
    let mut clob = Clob::new();
    let a = place_limit(&mut clob, Side::Buy, 100, 10);
    let b = place_limit(&mut clob, Side::Buy, 100, 5);

    let ev = modify(&mut clob, a, 100, 12);
    assert!(has_modified(&ev, a));
    assert_eq!(resting(&ev), Some((100, 12)));
    assert_eq!(clob.book().depth(Side::Buy, 1), vec![(100, 17)]);

    let sell = clob.submit(Command::New(NewOrder::limit(Side::Sell, 100, 5)));
    assert_eq!(trades(&sell), vec![(b, 100, 5)]);
    assert!(clob.book().contains(a));
    assert!(!clob.book().contains(b));
}

#[test]
fn price_change_moves_level_and_loses_priority() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Sell, 105, 4);
    let bid = place_limit(&mut clob, Side::Buy, 100, 10);

    let ev = modify(&mut clob, bid, 99, 10);
    assert!(trades(&ev).is_empty());
    assert_eq!(resting(&ev), Some((99, 10)));
    assert_eq!(clob.book().best_bid(), Some(99));
}

#[test]
fn price_up_crosses_and_rests_remainder_with_same_id() {
    let mut clob = Clob::new();
    let ask = place_limit(&mut clob, Side::Sell, 105, 4);
    let bid = place_limit(&mut clob, Side::Buy, 100, 10);

    let ev = modify(&mut clob, bid, 106, 10);
    assert!(has_modified(&ev, bid));
    assert_eq!(trades(&ev), vec![(ask, 105, 4)]);
    assert_eq!(resting(&ev), Some((106, 6)));
    assert!(clob.book().contains(bid));
    assert_eq!(clob.book().best_ask(), None);
    assert_eq!(clob.book().best_bid(), Some(106));
}

#[test]
fn modify_can_cross_and_fully_fill() {
    let mut clob = Clob::new();
    let ask = place_limit(&mut clob, Side::Sell, 105, 10);
    let bid = place_limit(&mut clob, Side::Buy, 100, 5);

    let ev = modify(&mut clob, bid, 106, 10);
    assert_eq!(trades(&ev), vec![(ask, 105, 10)]);
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::Filled { order_id, .. } if *order_id == bid))
    );
    assert!(!clob.book().contains(bid));
    assert!(clob.book().is_empty());
}

#[test]
fn modify_to_same_values_keeps_order_and_priority() {
    let mut clob = Clob::new();
    let a = place_limit(&mut clob, Side::Buy, 100, 10);
    let b = place_limit(&mut clob, Side::Buy, 100, 5);

    let ev = modify(&mut clob, a, 100, 10);
    assert!(has_modified(&ev, a));
    assert!(trades(&ev).is_empty());

    let sell = clob.submit(Command::New(NewOrder::limit(Side::Sell, 100, 10)));
    assert_eq!(trades(&sell), vec![(a, 100, 10)]);
    assert!(clob.book().contains(b));
}

#[test]
fn reduce_on_sell_side_keeps_priority() {
    let mut clob = Clob::new();
    let a = place_limit(&mut clob, Side::Sell, 101, 10);
    let b = place_limit(&mut clob, Side::Sell, 101, 5);

    modify(&mut clob, a, 101, 6);

    let buy = clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 6)));
    assert_eq!(trades(&buy), vec![(a, 101, 6)]);
    assert!(!clob.book().contains(a));
    assert!(clob.book().contains(b));
}

#[test]
fn modify_unknown_order_is_rejected() {
    let mut clob = Clob::new();
    let ev = modify(&mut clob, 999, 100, 1);
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::UnknownOrder,
            ..
        }
    )));
}

#[test]
fn modify_zero_qty_is_rejected_by_gateway() {
    let mut clob = Clob::new();
    let id = place_limit(&mut clob, Side::Buy, 100, 10);
    let ev = modify(&mut clob, id, 100, 0);
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::ZeroQuantity,
            ..
        }
    )));
    assert!(clob.book().contains(id));
}

#[test]
fn modify_zero_price_is_rejected_by_gateway() {
    let mut clob = Clob::new();
    let id = place_limit(&mut clob, Side::Buy, 100, 10);
    let ev = modify(&mut clob, id, 0, 5);
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::InvalidPrice,
            ..
        }
    )));
    assert!(clob.book().contains(id));
}
