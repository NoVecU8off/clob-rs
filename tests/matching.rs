mod common;

use clob::{Clob, Command, Event, NewOrder, RejectReason, Side, TimeInForce};

use common::{resting, trades};

#[test]
fn limit_order_rests_when_no_cross() {
    let mut clob = Clob::new();
    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 5)));
    assert!(trades(&events).is_empty());
    assert_eq!(resting(&events), Some((100, 5)));
    assert_eq!(clob.book().best_bid(), Some(100));
}

#[test]
fn price_time_priority_is_fifo_within_a_level() {
    let mut clob = Clob::new();
    let a = clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 5)));
    let b = clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 5)));
    let maker_a = match a[0] {
        Event::Accepted { order_id, .. } => order_id,
        _ => panic!("expected accepted"),
    };
    let maker_b = match b[0] {
        Event::Accepted { order_id, .. } => order_id,
        _ => panic!("expected accepted"),
    };

    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 8)));
    let t = trades(&events);
    assert_eq!(t, vec![(maker_a, 101, 5), (maker_b, 101, 3)]);
}

#[test]
fn best_price_matches_first_across_levels() {
    let mut clob = Clob::new();
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 102, 5)));
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 5)));

    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 105, 7)));
    let t = trades(&events);
    assert_eq!(t.len(), 2);
    assert_eq!(t[0].1, 101);
    assert_eq!(t[1].1, 102);
    assert_eq!(t[0].2 + t[1].2, 7);
}

#[test]
fn partial_fill_rests_remainder() {
    let mut clob = Clob::new();
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 4)));

    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 10)));
    assert_eq!(trades(&events), vec![(1, 101, 4)]);
    assert_eq!(resting(&events), Some((101, 6)));
    assert_eq!(clob.book().best_bid(), Some(101));
}

#[test]
fn ioc_cancels_unfilled_remainder() {
    let mut clob = Clob::new();
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 4)));

    let events = clob.submit(Command::New(
        NewOrder::limit(Side::Buy, 101, 10).with_tif(TimeInForce::Ioc),
    ));
    assert_eq!(trades(&events), vec![(1, 101, 4)]);
    assert!(resting(&events).is_none());
    assert!(events.iter().any(|e| matches!(e, Event::Canceled { .. })));
    assert!(clob.book().best_bid().is_none());
}

#[test]
fn fok_rejected_when_not_fully_fillable() {
    let mut clob = Clob::new();
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 4)));

    let events = clob.submit(Command::New(
        NewOrder::limit(Side::Buy, 101, 10).with_tif(TimeInForce::Fok),
    ));
    assert!(trades(&events).is_empty());
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::InsufficientLiquidity,
            ..
        }
    )));
    assert_eq!(clob.book().best_ask(), Some(101));
}

#[test]
fn fok_fully_fills_when_liquidity_is_present() {
    let mut clob = Clob::new();
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 4)));
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 102, 6)));

    let events = clob.submit(Command::New(
        NewOrder::limit(Side::Buy, 102, 10).with_tif(TimeInForce::Fok),
    ));
    assert_eq!(trades(&events).len(), 2);
    assert!(events.iter().any(|e| matches!(e, Event::Filled { .. })));
    assert!(clob.book().is_empty());
}

#[test]
fn market_order_sweeps_then_cancels_remainder() {
    let mut clob = Clob::new();
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 3)));
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 103, 3)));

    let events = clob.submit(Command::New(NewOrder::market(Side::Buy, 100)));
    assert_eq!(trades(&events).iter().map(|t| t.2).sum::<u64>(), 6);
    assert!(events.iter().any(|e| matches!(e, Event::Canceled { .. })));
    assert!(clob.book().is_empty());
}

#[test]
fn cancel_removes_resting_order() {
    let mut clob = Clob::new();
    let placed = clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 5)));
    let order_id = match placed[0] {
        Event::Accepted { order_id, .. } => order_id,
        _ => panic!("expected accepted"),
    };
    assert!(clob.book().contains(order_id));

    let events = clob.submit(Command::Cancel(clob::CancelOrder { order_id }));
    assert!(events.iter().any(|e| matches!(e, Event::Canceled { .. })));
    assert!(!clob.book().contains(order_id));
    assert!(clob.book().best_bid().is_none());
}

#[test]
fn cancel_unknown_order_is_rejected() {
    let mut clob = Clob::new();
    let events = clob.submit(Command::Cancel(clob::CancelOrder { order_id: 999 }));
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::UnknownOrder,
            ..
        }
    )));
}

#[test]
fn zero_quantity_is_rejected_by_gateway() {
    let mut clob = Clob::new();
    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 0)));
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::ZeroQuantity,
            ..
        }
    )));
}

#[test]
fn post_only_rejected_when_it_would_cross() {
    let mut clob = Clob::new();
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 5)));

    let events = clob.submit(Command::New(
        NewOrder::limit(Side::Buy, 102, 4).with_tif(TimeInForce::PostOnly),
    ));

    assert!(trades(&events).is_empty());
    assert!(resting(&events).is_none());
    assert!(!events.iter().any(|e| matches!(e, Event::Accepted { .. })));
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::WouldCross,
            ..
        }
    )));
    assert_eq!(clob.book().best_ask(), Some(101));
    assert!(clob.book().best_bid().is_none());
}

#[test]
fn post_only_rejected_at_touching_price() {
    let mut clob = Clob::new();
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 5)));

    let events = clob.submit(Command::New(
        NewOrder::limit(Side::Buy, 101, 4).with_tif(TimeInForce::PostOnly),
    ));

    assert!(events.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::WouldCross,
            ..
        }
    )));
    assert!(clob.book().best_bid().is_none());
}

#[test]
fn post_only_rests_when_it_would_not_cross() {
    let mut clob = Clob::new();
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 5)));

    let events = clob.submit(Command::New(
        NewOrder::limit(Side::Buy, 100, 4).with_tif(TimeInForce::PostOnly),
    ));

    assert!(trades(&events).is_empty());
    assert_eq!(resting(&events), Some((100, 4)));
    assert_eq!(clob.book().best_bid(), Some(100));
    assert_eq!(clob.book().best_ask(), Some(101));
}

#[test]
fn post_only_sell_rejected_when_it_would_cross() {
    let mut clob = Clob::new();
    clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 5)));

    let events = clob.submit(Command::New(
        NewOrder::limit(Side::Sell, 100, 4).with_tif(TimeInForce::PostOnly),
    ));

    assert!(trades(&events).is_empty());
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::WouldCross,
            ..
        }
    )));
    assert_eq!(clob.book().best_bid(), Some(100));
    assert!(clob.book().best_ask().is_none());
}

#[test]
fn post_only_rests_on_empty_book() {
    let mut clob = Clob::new();
    let events = clob.submit(Command::New(
        NewOrder::limit(Side::Buy, 100, 5).with_tif(TimeInForce::PostOnly),
    ));

    assert!(trades(&events).is_empty());
    assert_eq!(resting(&events), Some((100, 5)));
    assert_eq!(clob.book().best_bid(), Some(100));
}

#[test]
fn post_only_market_is_rejected() {
    let mut clob = Clob::new();
    let events = clob.submit(Command::New(
        NewOrder::market(Side::Buy, 5).with_tif(TimeInForce::PostOnly),
    ));

    assert!(trades(&events).is_empty());
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::WouldCross,
            ..
        }
    )));
    assert!(clob.book().is_empty());
}

#[test]
fn book_is_never_crossed_after_matching() {
    let mut clob = Clob::new();
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 5)));
    clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 5)));
    clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 2)));

    let book = clob.book();
    if let (Some(bid), Some(ask)) = (book.best_bid(), book.best_ask()) {
        assert!(ask > bid, "book must not be crossed: bid={bid} ask={ask}");
    }
}
