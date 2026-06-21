mod common;

use clob::{
    CancelOrder, Clob, Command, Event, ModifyOrder, NewOrder, PersistentClob, RejectReason,
    RiskConfig, Side,
};

use common::{TempFile, accepted_id};

fn rejected(events: &[Event]) -> Option<RejectReason> {
    events.iter().find_map(|e| match e {
        Event::Rejected { reason, .. } => Some(*reason),
        _ => None,
    })
}

fn is_accepted(events: &[Event]) -> bool {
    events.iter().any(|e| matches!(e, Event::Accepted { .. }))
}

fn buy(owner: u64, price: u64, qty: u64) -> Command {
    Command::New(NewOrder::limit(Side::Buy, price, qty).with_owner(owner))
}

fn sell(owner: u64, price: u64, qty: u64) -> Command {
    Command::New(NewOrder::limit(Side::Sell, price, qty).with_owner(owner))
}

#[test]
fn tick_rejects_misaligned_limit() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_tick(10));
    assert_eq!(
        rejected(&clob.submit(buy(0, 105, 5))),
        Some(RejectReason::TickSize)
    );
    assert!(is_accepted(&clob.submit(buy(0, 100, 5))));
}

#[test]
fn tick_checks_stop_and_stop_limit() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_tick(10));
    let ev = clob.submit(Command::New(NewOrder::stop(Side::Buy, 105, 5)));
    assert_eq!(rejected(&ev), Some(RejectReason::TickSize));
    let ev = clob.submit(Command::New(NewOrder::stop_limit(Side::Buy, 100, 105, 5)));
    assert_eq!(rejected(&ev), Some(RejectReason::TickSize));
    let ev = clob.submit(Command::New(NewOrder::stop_limit(Side::Buy, 100, 110, 5)));
    assert!(is_accepted(&ev));
}

#[test]
fn tick_allows_market_without_price() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_tick(10));
    let ev = clob.submit(Command::New(NewOrder::market(Side::Buy, 5)));
    assert!(is_accepted(&ev));
}

#[test]
fn lot_rejects_misaligned_qty() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_lot(5));
    assert_eq!(
        rejected(&clob.submit(buy(0, 100, 7))),
        Some(RejectReason::LotSize)
    );
    assert!(is_accepted(&clob.submit(buy(0, 100, 10))));
}

#[test]
fn lot_checks_iceberg_display() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_lot(5));
    let ev = clob.submit(Command::New(NewOrder::iceberg(Side::Sell, 100, 20, 7)));
    assert_eq!(rejected(&ev), Some(RejectReason::LotSize));
    let ev = clob.submit(Command::New(NewOrder::iceberg(Side::Sell, 100, 20, 5)));
    assert!(is_accepted(&ev));
}

#[test]
fn corridor_rejects_outside_band() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_price_band(10));
    clob.submit(sell(0, 110, 5));
    clob.submit(buy(0, 90, 5));
    assert_eq!(
        rejected(&clob.submit(buy(0, 111, 1))),
        Some(RejectReason::PriceBand)
    );
    assert_eq!(
        rejected(&clob.submit(sell(0, 89, 1))),
        Some(RejectReason::PriceBand)
    );
    assert!(is_accepted(&clob.submit(buy(0, 100, 1))));
}

#[test]
fn corridor_skips_on_cold_book() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_price_band(5));
    assert!(is_accepted(&clob.submit(buy(0, 1000, 5))));
}

#[test]
fn corridor_offset_scales_with_tick() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_tick(10).with_price_band(2));
    clob.submit(sell(0, 110, 5));
    clob.submit(buy(0, 90, 5));
    assert_eq!(
        rejected(&clob.submit(buy(0, 130, 10))),
        Some(RejectReason::PriceBand)
    );
    assert!(is_accepted(&clob.submit(buy(0, 100, 10))));
}

#[test]
fn corridor_exempts_stops() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_price_band(10));
    clob.submit(sell(0, 110, 5));
    clob.submit(buy(0, 90, 5));
    let ev = clob.submit(Command::New(NewOrder::stop(Side::Buy, 200, 5)));
    assert!(is_accepted(&ev));
    assert_eq!(clob.pending_stops(), 1);
}

#[test]
fn corridor_exempts_market() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_price_band(10));
    clob.submit(sell(0, 110, 5));
    clob.submit(buy(0, 90, 5));
    let ev = clob.submit(Command::New(NewOrder::market(Side::Buy, 3)));
    assert!(is_accepted(&ev));
}

#[test]
fn position_limit_caps_open_exposure() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_position_limit(100));
    assert!(is_accepted(&clob.submit(buy(1, 50, 60))));
    assert!(is_accepted(&clob.submit(buy(1, 50, 40))));
    assert_eq!(
        rejected(&clob.submit(buy(1, 50, 1))),
        Some(RejectReason::PositionLimit)
    );
}

#[test]
fn position_limit_counts_realized_net() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_position_limit(100));
    clob.submit(sell(2, 50, 60));
    let ev = clob.submit(buy(1, 50, 60));
    assert!(is_accepted(&ev));
    assert!(clob.book().is_empty());
    assert_eq!(
        rejected(&clob.submit(buy(1, 50, 41))),
        Some(RejectReason::PositionLimit)
    );
    assert!(is_accepted(&clob.submit(buy(1, 50, 40))));
}

#[test]
fn position_worst_case_combines_net_and_open() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_position_limit(100));
    clob.submit(buy(1, 50, 50));
    clob.submit(sell(2, 60, 30));
    let ev = clob.submit(buy(1, 60, 30));
    assert!(is_accepted(&ev));
    assert_eq!(
        rejected(&clob.submit(buy(1, 50, 21))),
        Some(RejectReason::PositionLimit)
    );
    assert!(is_accepted(&clob.submit(buy(1, 50, 20))));
}

#[test]
fn position_limit_symmetric_for_sells() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_position_limit(100));
    assert!(is_accepted(&clob.submit(sell(1, 50, 60))));
    assert_eq!(
        rejected(&clob.submit(sell(1, 50, 41))),
        Some(RejectReason::PositionLimit)
    );
}

#[test]
fn position_exempts_anonymous() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_position_limit(10));
    assert!(is_accepted(&clob.submit(buy(0, 50, 1000))));
}

#[test]
fn cancel_frees_position_exposure() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_position_limit(100));
    let id = accepted_id(&clob.submit(buy(1, 50, 100)));
    assert_eq!(
        rejected(&clob.submit(buy(1, 50, 1))),
        Some(RejectReason::PositionLimit)
    );
    clob.submit(Command::Cancel(CancelOrder { order_id: id }));
    assert!(is_accepted(&clob.submit(buy(1, 50, 100))));
}

#[test]
fn modify_uses_own_adjusted_exposure() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_position_limit(100));
    let id = accepted_id(&clob.submit(buy(1, 50, 80)));
    let ev = clob.submit(Command::Modify(ModifyOrder::new(id, 50, 100)));
    assert!(ev.iter().any(|e| matches!(e, Event::Modified { .. })));
    assert!(rejected(&ev).is_none());
    let ev = clob.submit(Command::Modify(ModifyOrder::new(id, 50, 101)));
    assert_eq!(rejected(&ev), Some(RejectReason::PositionLimit));
    assert_eq!(clob.book().level_qty(Side::Buy, 50), 100);
}

#[test]
fn position_survives_checkpoint() {
    let tmp = TempFile::new("risk_checkpoint");
    let cfg = RiskConfig::new().with_position_limit(100);
    {
        let mut p = PersistentClob::open_with_risk(tmp.path(), cfg).unwrap();
        p.submit(sell(2, 50, 60)).unwrap();
        p.submit(buy(1, 50, 60)).unwrap();
        p.checkpoint().unwrap();
    }
    let mut p = PersistentClob::open_with_risk(tmp.path(), cfg).unwrap();
    assert_eq!(
        rejected(&p.submit(buy(1, 50, 41)).unwrap()),
        Some(RejectReason::PositionLimit)
    );
    assert!(is_accepted(&p.submit(buy(1, 50, 40)).unwrap()));
}

#[test]
fn position_rebuilt_by_replay() {
    let tmp = TempFile::new("risk_replay");
    let cfg = RiskConfig::new().with_position_limit(100);
    {
        let mut p = PersistentClob::open_with_risk(tmp.path(), cfg).unwrap();
        p.submit(sell(2, 50, 60)).unwrap();
        p.submit(buy(1, 50, 60)).unwrap();
    }
    let mut p = PersistentClob::open_with_risk(tmp.path(), cfg).unwrap();
    assert_eq!(
        rejected(&p.submit(buy(1, 50, 41)).unwrap()),
        Some(RejectReason::PositionLimit)
    );
}

#[test]
fn iceberg_reserve_counts_toward_exposure() {
    let mut clob = Clob::with_risk(RiskConfig::new().with_position_limit(100));
    assert!(is_accepted(&clob.submit(Command::New(
        NewOrder::iceberg(Side::Sell, 50, 80, 10).with_owner(1)
    ))));
    assert_eq!(
        rejected(&clob.submit(sell(1, 60, 21))),
        Some(RejectReason::PositionLimit)
    );
    assert!(is_accepted(&clob.submit(sell(1, 60, 20))));
}

#[test]
fn stp_self_cancel_frees_maker_exposure() {
    use clob::StpMode;
    let mut clob = Clob::with_risk(RiskConfig::new().with_position_limit(100));
    clob.submit(sell(1, 50, 60));
    clob.submit(Command::New(
        NewOrder::limit(Side::Buy, 50, 30)
            .with_owner(1)
            .with_stp(StpMode::CancelMaker),
    ));
    assert!(is_accepted(&clob.submit(sell(1, 60, 100))));
}

#[test]
fn defaults_off_accept_everything() {
    let mut clob = Clob::new();
    assert!(is_accepted(&clob.submit(buy(1, 105, 7))));
    assert!(is_accepted(&clob.submit(buy(1, 50, 1_000_000))));
}
