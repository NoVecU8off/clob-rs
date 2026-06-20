mod common;

use clob::{CancelOrder, Clob, Command, Event, NewOrder, RejectReason, Side};

use common::{place_limit, trades};

fn cancel(clob: &mut Clob, order_id: u64) -> Vec<Event> {
    clob.submit(Command::Cancel(CancelOrder { order_id }))
}

#[test]
fn cancel_head_preserves_fifo_of_remaining() {
    let mut clob = Clob::new();
    let a = place_limit(&mut clob, Side::Sell, 101, 3);
    let b = place_limit(&mut clob, Side::Sell, 101, 3);
    let c = place_limit(&mut clob, Side::Sell, 101, 3);

    cancel(&mut clob, a);
    let buy = clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 6)));
    assert_eq!(trades(&buy), vec![(b, 101, 3), (c, 101, 3)]);
}

#[test]
fn cancel_middle_preserves_fifo_of_remaining() {
    let mut clob = Clob::new();
    let a = place_limit(&mut clob, Side::Sell, 101, 3);
    let b = place_limit(&mut clob, Side::Sell, 101, 3);
    let c = place_limit(&mut clob, Side::Sell, 101, 3);

    cancel(&mut clob, b);
    let buy = clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 6)));
    assert_eq!(trades(&buy), vec![(a, 101, 3), (c, 101, 3)]);
}

#[test]
fn cancel_tail_preserves_fifo_of_remaining() {
    let mut clob = Clob::new();
    let a = place_limit(&mut clob, Side::Sell, 101, 3);
    let b = place_limit(&mut clob, Side::Sell, 101, 3);
    let c = place_limit(&mut clob, Side::Sell, 101, 3);

    cancel(&mut clob, c);
    let buy = clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 6)));
    assert_eq!(trades(&buy), vec![(a, 101, 3), (b, 101, 3)]);
}

#[test]
fn cancel_only_order_removes_price_level() {
    let mut clob = Clob::new();
    let a = place_limit(&mut clob, Side::Buy, 100, 5);

    cancel(&mut clob, a);
    assert!(clob.book().is_empty());
    assert_eq!(clob.book().best_bid(), None);
    assert!(clob.book().depth(Side::Buy, 5).is_empty());
}

#[test]
fn cancel_updates_level_total_qty() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Buy, 100, 10);
    let b = place_limit(&mut clob, Side::Buy, 100, 7);
    assert_eq!(clob.book().depth(Side::Buy, 1), vec![(100, 17)]);

    cancel(&mut clob, b);
    assert_eq!(clob.book().depth(Side::Buy, 1), vec![(100, 10)]);
}

#[test]
fn cancel_does_not_affect_other_levels() {
    let mut clob = Clob::new();
    let a = place_limit(&mut clob, Side::Buy, 100, 5);
    place_limit(&mut clob, Side::Buy, 99, 8);

    cancel(&mut clob, a);
    assert_eq!(clob.book().best_bid(), Some(99));
    assert_eq!(clob.book().depth(Side::Buy, 5), vec![(99, 8)]);
}

#[test]
fn cancel_twice_second_is_rejected() {
    let mut clob = Clob::new();
    let a = place_limit(&mut clob, Side::Buy, 100, 5);

    cancel(&mut clob, a);
    let ev = cancel(&mut clob, a);
    assert!(ev.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::UnknownOrder,
            ..
        }
    )));
}

#[test]
fn churned_insert_cancel_keeps_book_correct() {
    let mut clob = Clob::new();
    for _ in 0..50 {
        let id = place_limit(&mut clob, Side::Buy, 100, 1);
        cancel(&mut clob, id);
    }
    assert!(clob.book().is_empty());

    let a = place_limit(&mut clob, Side::Sell, 101, 4);
    let b = place_limit(&mut clob, Side::Sell, 101, 4);
    let buy = clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 6)));
    assert_eq!(trades(&buy), vec![(a, 101, 4), (b, 101, 2)]);
    assert!(clob.book().contains(b));
}
