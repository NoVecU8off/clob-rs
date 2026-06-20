mod common;

use clob::{Clob, Command, Event, NewOrder, PersistentClob, Side, StpMode};

use common::{TempFile, resting, trades};

fn canceled(events: &[Event]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Canceled { order_id, .. } => Some(*order_id),
            _ => None,
        })
        .collect()
}

fn sell(owner: u64, price: u64, qty: u64) -> Command {
    Command::New(NewOrder::limit(Side::Sell, price, qty).with_owner(owner))
}

fn buy_stp(owner: u64, price: u64, qty: u64, stp: StpMode) -> Command {
    Command::New(
        NewOrder::limit(Side::Buy, price, qty)
            .with_owner(owner)
            .with_stp(stp),
    )
}

#[test]
fn cancel_taker_rejects_self_cross() {
    let mut clob = Clob::new();
    clob.submit(sell(1, 100, 5));
    let ev = clob.submit(buy_stp(1, 100, 3, StpMode::CancelTaker));
    assert!(trades(&ev).is_empty());
    assert_eq!(canceled(&ev), vec![2]);
    assert_eq!(clob.book().best_ask(), Some(100));
    assert_eq!(clob.book().level_qty(Side::Sell, 100), 5);
    assert_eq!(clob.book().len(), 1);
}

#[test]
fn cancel_taker_trades_other_owners_then_stops() {
    let mut clob = Clob::new();
    clob.submit(sell(2, 100, 4));
    clob.submit(sell(1, 100, 5));
    let ev = clob.submit(buy_stp(1, 100, 12, StpMode::CancelTaker));
    assert_eq!(trades(&ev), vec![(1, 100, 4)]);
    assert_eq!(canceled(&ev), vec![3]);
    assert_eq!(clob.book().level_qty(Side::Sell, 100), 5);
    assert_eq!(clob.book().len(), 1);
}

#[test]
fn cancel_maker_pulls_resting_and_taker_rests() {
    let mut clob = Clob::new();
    clob.submit(sell(1, 100, 5));
    let ev = clob.submit(buy_stp(1, 100, 3, StpMode::CancelMaker));
    assert!(trades(&ev).is_empty());
    assert_eq!(canceled(&ev), vec![1]);
    assert_eq!(resting(&ev), Some((100, 3)));
    assert_eq!(clob.book().best_bid(), Some(100));
    assert_eq!(clob.book().best_ask(), None);
    assert_eq!(clob.book().len(), 1);
}

#[test]
fn cancel_maker_then_trades_next_owner() {
    let mut clob = Clob::new();
    clob.submit(sell(1, 100, 5));
    clob.submit(sell(2, 100, 4));
    let ev = clob.submit(buy_stp(1, 100, 6, StpMode::CancelMaker));
    assert_eq!(trades(&ev), vec![(2, 100, 4)]);
    assert_eq!(canceled(&ev), vec![1]);
    assert_eq!(resting(&ev), Some((100, 2)));
    assert_eq!(clob.book().best_bid(), Some(100));
    assert_eq!(clob.book().best_ask(), None);
}

#[test]
fn cancel_both_pulls_maker_and_taker() {
    let mut clob = Clob::new();
    clob.submit(sell(1, 100, 5));
    let ev = clob.submit(buy_stp(1, 100, 3, StpMode::CancelBoth));
    assert!(trades(&ev).is_empty());
    assert_eq!(canceled(&ev), vec![1, 2]);
    assert!(clob.book().is_empty());
}

#[test]
fn stp_does_not_fire_across_owners() {
    let mut clob = Clob::new();
    clob.submit(sell(1, 100, 5));
    let ev = clob.submit(buy_stp(2, 100, 3, StpMode::CancelBoth));
    assert_eq!(trades(&ev), vec![(1, 100, 3)]);
    assert!(canceled(&ev).is_empty());
    assert_eq!(clob.book().level_qty(Side::Sell, 100), 2);
}

#[test]
fn anonymous_taker_ignores_stp() {
    let mut clob = Clob::new();
    clob.submit(sell(0, 100, 5));
    let ev = clob.submit(Command::New(
        NewOrder::limit(Side::Buy, 100, 3).with_stp(StpMode::CancelBoth),
    ));
    assert_eq!(trades(&ev), vec![(1, 100, 3)]);
    assert!(canceled(&ev).is_empty());
}

#[test]
fn same_owner_stp_off_allows_self_trade() {
    let mut clob = Clob::new();
    clob.submit(sell(1, 100, 5));
    let ev = clob.submit(Command::New(
        NewOrder::limit(Side::Buy, 100, 3).with_owner(1),
    ));
    assert_eq!(trades(&ev), vec![(1, 100, 3)]);
    assert!(canceled(&ev).is_empty());
    assert_eq!(clob.book().level_qty(Side::Sell, 100), 2);
}

#[test]
fn triggered_stop_applies_stp() {
    let mut clob = Clob::new();
    clob.submit(sell(1, 100, 5));
    clob.submit(Command::New(
        NewOrder::stop(Side::Buy, 100, 3)
            .with_owner(1)
            .with_stp(StpMode::CancelMaker),
    ));
    assert_eq!(clob.pending_stops(), 1);
    let ev = clob.submit(Command::New(
        NewOrder::limit(Side::Buy, 100, 1).with_owner(2),
    ));
    assert_eq!(trades(&ev), vec![(1, 100, 1)]);
    assert!(
        ev.iter()
            .any(|e| matches!(e, Event::Triggered { order_id: 2, .. }))
    );
    assert!(canceled(&ev).contains(&1));
    assert_eq!(clob.pending_stops(), 0);
    assert!(clob.book().is_empty());
}

#[test]
fn owner_survives_snapshot_and_replay() {
    let tmp = TempFile::new("stp");
    {
        let mut p = PersistentClob::open(tmp.path()).unwrap();
        p.submit(sell(1, 100, 5)).unwrap();
        p.checkpoint().unwrap();
        p.submit(buy_stp(1, 100, 3, StpMode::CancelMaker)).unwrap();
        assert_eq!(p.book().best_bid(), Some(100));
        assert_eq!(p.book().best_ask(), None);
        assert_eq!(p.book().level_qty(Side::Buy, 100), 3);
    }
    let p = PersistentClob::open(tmp.path()).unwrap();
    assert_eq!(p.book().best_bid(), Some(100));
    assert_eq!(p.book().best_ask(), None);
    assert_eq!(p.book().level_qty(Side::Buy, 100), 3);
}
