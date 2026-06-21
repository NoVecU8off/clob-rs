mod common;

use clob::{
    Clob, Command, Event, FeeConfig, NewOrder, PersistentClob, RejectReason, RiskConfig, Side,
};

use common::TempFile;

fn trade_fees(events: &[Event]) -> Vec<(i64, i64)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Trade {
                taker_fee,
                maker_fee,
                ..
            } => Some((*taker_fee, *maker_fee)),
            _ => None,
        })
        .collect()
}

fn rejected(events: &[Event]) -> Option<RejectReason> {
    events.iter().find_map(|e| match e {
        Event::Rejected { reason, .. } => Some(*reason),
        _ => None,
    })
}

fn sell(clob: &mut Clob, price: u64, qty: u64) {
    clob.submit(Command::New(NewOrder::limit(Side::Sell, price, qty)));
}

fn buy(clob: &mut Clob, price: u64, qty: u64) -> Vec<Event> {
    clob.submit(Command::New(NewOrder::limit(Side::Buy, price, qty)))
}

#[test]
fn default_clob_charges_no_fees() {
    let mut clob = Clob::new();
    sell(&mut clob, 1000, 100);
    assert_eq!(trade_fees(&buy(&mut clob, 1000, 100)), vec![(0, 0)]);
}

#[test]
fn taker_fee_only() {
    let mut clob = Clob::with_fees(FeeConfig::new().with_taker_ppm(2000));
    sell(&mut clob, 1000, 100);
    assert_eq!(trade_fees(&buy(&mut clob, 1000, 100)), vec![(200, 0)]);
}

#[test]
fn maker_and_taker_fee() {
    let mut clob = Clob::with_fees(FeeConfig::new().with_taker_ppm(2000).with_maker_ppm(1000));
    sell(&mut clob, 1000, 100);
    assert_eq!(trade_fees(&buy(&mut clob, 1000, 100)), vec![(200, 100)]);
}

#[test]
fn maker_rebate_is_negative() {
    let mut clob = Clob::with_fees(FeeConfig::new().with_taker_ppm(2000).with_maker_ppm(-1000));
    sell(&mut clob, 1000, 100);
    assert_eq!(trade_fees(&buy(&mut clob, 1000, 100)), vec![(200, -100)]);
}

#[test]
fn fee_truncates_toward_zero() {
    let mut clob = Clob::with_fees(FeeConfig::new().with_taker_ppm(2000).with_maker_ppm(-2000));
    sell(&mut clob, 70, 10);
    assert_eq!(trade_fees(&buy(&mut clob, 70, 10)), vec![(1, -1)]);
}

#[test]
fn sub_unit_fee_rounds_to_zero() {
    let mut clob = Clob::with_fees(FeeConfig::new().with_taker_ppm(2000));
    sell(&mut clob, 10, 10);
    assert_eq!(trade_fees(&buy(&mut clob, 10, 10)), vec![(0, 0)]);
}

#[test]
fn fee_charged_per_fill_across_levels() {
    let mut clob = Clob::with_fees(FeeConfig::new().with_taker_ppm(2000));
    sell(&mut clob, 1000, 50);
    sell(&mut clob, 1001, 50);
    assert_eq!(
        trade_fees(&buy(&mut clob, 1001, 100)),
        vec![(100, 0), (100, 0)]
    );
}

#[test]
fn market_taker_pays_fee_at_execution_price() {
    let mut clob = Clob::with_fees(FeeConfig::new().with_taker_ppm(2000));
    sell(&mut clob, 1000, 100);
    let ev = clob.submit(Command::New(NewOrder::market(Side::Buy, 100)));
    assert_eq!(trade_fees(&ev), vec![(200, 0)]);
}

#[test]
fn iceberg_fees_cover_hidden_volume() {
    let mut clob = Clob::with_fees(FeeConfig::new().with_taker_ppm(2000));
    clob.submit(Command::New(NewOrder::iceberg(Side::Sell, 1000, 100, 20)));
    let fees = trade_fees(&buy(&mut clob, 1000, 100));
    assert_eq!(fees.iter().map(|(t, _)| t).sum::<i64>(), 200);
    assert!(fees.len() > 1);
}

#[test]
fn fees_apply_through_persistent_clob_replay() {
    let file = TempFile::new("fees");
    let cfg = FeeConfig::new().with_taker_ppm(2000).with_maker_ppm(1000);
    {
        let mut clob = PersistentClob::open_with_fees(file.path(), cfg).unwrap();
        clob.submit(Command::New(NewOrder::limit(Side::Sell, 1000, 60)))
            .unwrap();
        let ev = clob
            .submit(Command::New(NewOrder::limit(Side::Buy, 1000, 60)))
            .unwrap();
        assert_eq!(trade_fees(&ev), vec![(120, 60)]);
    }
    let mut clob = PersistentClob::open_with_fees(file.path(), cfg).unwrap();
    clob.submit(Command::New(NewOrder::limit(Side::Sell, 1000, 50)))
        .unwrap();
    let ev = clob
        .submit(Command::New(NewOrder::limit(Side::Buy, 1000, 50)))
        .unwrap();
    assert_eq!(trade_fees(&ev), vec![(100, 50)]);
}

#[test]
fn risk_and_fees_compose() {
    let risk = RiskConfig::new().with_lot(5);
    let cfg = FeeConfig::new().with_taker_ppm(2000).with_maker_ppm(1000);
    let mut clob = Clob::with_risk_and_fees(risk, cfg);
    assert_eq!(
        rejected(&clob.submit(Command::New(NewOrder::limit(Side::Buy, 1000, 7)))),
        Some(RejectReason::LotSize)
    );
    sell(&mut clob, 1000, 100);
    assert_eq!(trade_fees(&buy(&mut clob, 1000, 100)), vec![(200, 100)]);
}
