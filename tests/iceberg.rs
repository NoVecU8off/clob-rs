mod common;

use clob::{
    CancelOrder, Clob, Command, Event, ModifyOrder, NewOrder, RejectReason, Side, TimeInForce,
};

use common::{accepted_id, resting, trades};

#[test]
fn iceberg_rests_showing_only_the_peak() {
    let mut clob = Clob::new();
    let events = clob.submit(Command::New(NewOrder::iceberg(Side::Buy, 100, 10, 3)));
    assert!(trades(&events).is_empty());
    assert_eq!(resting(&events), Some((100, 3)));
    assert_eq!(clob.book().depth(Side::Buy, 1), vec![(100, 3)]);
    assert_eq!(clob.book().best_bid(), Some(100));
    assert_eq!(clob.book().len(), 1);
}

#[test]
fn peak_refills_after_being_consumed() {
    let mut clob = Clob::new();
    let ice = accepted_id(&clob.submit(Command::New(NewOrder::iceberg(Side::Sell, 101, 10, 3))));

    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 3)));
    assert_eq!(trades(&events), vec![(ice, 101, 3)]);
    assert_eq!(clob.book().depth(Side::Sell, 1), vec![(101, 3)]);
    assert_eq!(clob.book().len(), 1);
}

#[test]
fn large_aggressor_sweeps_entire_iceberg() {
    let mut clob = Clob::new();
    let ice = accepted_id(&clob.submit(Command::New(NewOrder::iceberg(Side::Sell, 101, 10, 3))));

    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 10)));
    let t = trades(&events);
    assert_eq!(
        t,
        vec![(ice, 101, 3), (ice, 101, 3), (ice, 101, 3), (ice, 101, 1)]
    );
    assert_eq!(t.iter().map(|x| x.2).sum::<u64>(), 10);
    assert!(events.iter().any(|e| matches!(e, Event::Filled { .. })));
    assert!(clob.book().is_empty());
}

#[test]
fn refill_loses_priority_to_displayed_orders() {
    let mut clob = Clob::new();
    let ice = accepted_id(&clob.submit(Command::New(NewOrder::iceberg(Side::Sell, 101, 10, 2))));
    let plain = accepted_id(&clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 5))));

    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 4)));
    assert_eq!(trades(&events), vec![(ice, 101, 2), (plain, 101, 2)]);
    assert_eq!(clob.book().depth(Side::Sell, 1), vec![(101, 5)]);
}

#[test]
fn final_slice_can_be_smaller_than_display() {
    let mut clob = Clob::new();
    let ice = accepted_id(&clob.submit(Command::New(NewOrder::iceberg(Side::Sell, 101, 7, 3))));

    clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 3)));
    assert_eq!(clob.book().depth(Side::Sell, 1), vec![(101, 3)]);

    clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 3)));
    assert_eq!(clob.book().depth(Side::Sell, 1), vec![(101, 1)]);

    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 1)));
    assert_eq!(trades(&events), vec![(ice, 101, 1)]);
    assert!(clob.book().is_empty());
}

#[test]
fn iceberg_as_taker_crosses_then_rests_remainder() {
    let mut clob = Clob::new();
    let maker = accepted_id(&clob.submit(Command::New(NewOrder::limit(Side::Sell, 100, 4))));

    let events = clob.submit(Command::New(NewOrder::iceberg(Side::Buy, 100, 10, 3)));
    assert_eq!(trades(&events), vec![(maker, 100, 4)]);
    assert_eq!(resting(&events), Some((100, 3)));
    assert_eq!(clob.book().depth(Side::Buy, 1), vec![(100, 3)]);
    assert!(clob.book().best_ask().is_none());
}

#[test]
fn post_only_iceberg_rejected_when_it_would_cross() {
    let mut clob = Clob::new();
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 5)));

    let events = clob.submit(Command::New(
        NewOrder::iceberg(Side::Buy, 101, 10, 3).with_tif(TimeInForce::PostOnly),
    ));
    assert!(trades(&events).is_empty());
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::WouldCross,
            ..
        }
    )));
    assert!(clob.book().best_bid().is_none());
    assert_eq!(clob.book().best_ask(), Some(101));
}

#[test]
fn post_only_iceberg_rests_when_it_would_not_cross() {
    let mut clob = Clob::new();
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 101, 5)));

    let events = clob.submit(Command::New(
        NewOrder::iceberg(Side::Buy, 100, 10, 3).with_tif(TimeInForce::PostOnly),
    ));
    assert!(trades(&events).is_empty());
    assert_eq!(resting(&events), Some((100, 3)));
    assert_eq!(clob.book().depth(Side::Buy, 1), vec![(100, 3)]);
}

#[test]
fn fok_taker_ignores_hidden_reserve() {
    let mut clob = Clob::new();
    clob.submit(Command::New(NewOrder::iceberg(Side::Sell, 101, 10, 2)));

    let rejected = clob.submit(Command::New(
        NewOrder::limit(Side::Buy, 101, 5).with_tif(TimeInForce::Fok),
    ));
    assert!(trades(&rejected).is_empty());
    assert!(rejected.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::InsufficientLiquidity,
            ..
        }
    )));
    assert_eq!(clob.book().depth(Side::Sell, 1), vec![(101, 2)]);

    let filled = clob.submit(Command::New(
        NewOrder::limit(Side::Buy, 101, 2).with_tif(TimeInForce::Fok),
    ));
    assert_eq!(trades(&filled).iter().map(|t| t.2).sum::<u64>(), 2);
    assert!(filled.iter().any(|e| matches!(e, Event::Filled { .. })));
}

#[test]
fn display_not_less_than_qty_rests_fully_visible() {
    let mut clob = Clob::new();
    let ice = accepted_id(&clob.submit(Command::New(NewOrder::iceberg(Side::Buy, 100, 5, 5))));
    assert_eq!(clob.book().depth(Side::Buy, 1), vec![(100, 5)]);

    let events = clob.submit(Command::New(NewOrder::limit(Side::Sell, 100, 5)));
    assert_eq!(trades(&events), vec![(ice, 100, 5)]);
    assert!(clob.book().is_empty());
}

#[test]
fn cancel_removes_iceberg() {
    let mut clob = Clob::new();
    let ice = accepted_id(&clob.submit(Command::New(NewOrder::iceberg(Side::Buy, 100, 10, 3))));
    assert!(clob.book().contains(ice));

    let events = clob.submit(Command::Cancel(CancelOrder { order_id: ice }));
    assert!(events.iter().any(|e| matches!(e, Event::Canceled { .. })));
    assert!(!clob.book().contains(ice));
    assert!(clob.book().is_empty());
}

#[test]
fn modify_iceberg_resizes_total_and_preserves_display() {
    let mut clob = Clob::new();
    let ice = accepted_id(&clob.submit(Command::New(NewOrder::iceberg(Side::Sell, 101, 10, 3))));

    let events = clob.submit(Command::Modify(ModifyOrder::new(ice, 101, 6)));
    assert!(events.iter().any(|e| matches!(e, Event::Modified { .. })));
    assert_eq!(resting(&events), Some((101, 3)));

    let swept = clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 6)));
    assert_eq!(trades(&swept), vec![(ice, 101, 3), (ice, 101, 3)]);
    assert!(clob.book().is_empty());
}

#[test]
fn modify_iceberg_reprices_and_keeps_hidden() {
    let mut clob = Clob::new();
    let ice = accepted_id(&clob.submit(Command::New(NewOrder::iceberg(Side::Sell, 101, 10, 3))));

    let events = clob.submit(Command::Modify(ModifyOrder::new(ice, 102, 10)));
    assert_eq!(resting(&events), Some((102, 3)));
    assert_eq!(clob.book().best_ask(), Some(102));

    let swept = clob.submit(Command::New(NewOrder::limit(Side::Buy, 102, 10)));
    assert_eq!(trades(&swept).iter().map(|t| t.2).sum::<u64>(), 10);
    assert_eq!(trades(&swept).len(), 4);
    assert!(clob.book().is_empty());
}

#[test]
fn gateway_rejects_zero_display() {
    let mut clob = Clob::new();
    let events = clob.submit(Command::New(NewOrder::iceberg(Side::Buy, 100, 10, 0)));
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::ZeroQuantity,
            ..
        }
    )));
    assert!(clob.book().is_empty());
}

#[test]
fn gateway_rejects_zero_price_iceberg() {
    let mut clob = Clob::new();
    let events = clob.submit(Command::New(NewOrder::iceberg(Side::Buy, 0, 10, 3)));
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::InvalidPrice,
            ..
        }
    )));
    assert!(clob.book().is_empty());
}

#[test]
fn iceberg_flow_is_deterministic() {
    let commands = || {
        vec![
            Command::New(NewOrder::iceberg(Side::Sell, 101, 10, 3)),
            Command::New(NewOrder::iceberg(Side::Sell, 101, 8, 2)),
            Command::New(NewOrder::limit(Side::Sell, 101, 5)),
            Command::New(NewOrder::limit(Side::Buy, 101, 9)),
            Command::New(NewOrder::limit(Side::Buy, 101, 6)),
            Command::Modify(ModifyOrder::new(1, 101, 4)),
        ]
    };

    let run = |cmds: Vec<Command>| {
        let mut clob = Clob::new();
        let mut events = Vec::new();
        for c in cmds {
            events.extend(clob.submit(c));
        }
        events
    };

    assert_eq!(run(commands()), run(commands()));
}
