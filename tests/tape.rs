mod common;

use clob::{Clob, Command, Event, NewOrder, Side, TapeTrade, TimeInForce, TradeTape};
use common::{place_limit, script};

fn prints(events: &[Event]) -> Vec<TapeTrade> {
    events
        .iter()
        .filter_map(|e| match *e {
            Event::Trade {
                seq,
                price,
                qty,
                taker_side,
                ..
            } => Some(TapeTrade {
                seq,
                price,
                qty,
                taker_side,
            }),
            _ => None,
        })
        .collect()
}

#[test]
fn cross_emits_one_print() {
    let mut clob = Clob::new();
    let mut tape = TradeTape::new();
    place_limit(&mut clob, Side::Sell, 101, 10);

    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 4)));
    let out = tape.apply(&events);

    assert_eq!(
        out,
        vec![TapeTrade {
            seq: 2,
            price: 101,
            qty: 4,
            taker_side: Side::Buy,
        }]
    );
    assert_eq!(tape.last(), out.first());
}

#[test]
fn non_trade_commands_print_nothing() {
    let mut clob = Clob::new();
    let mut tape = TradeTape::new();

    let resting = clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 5)));
    assert!(tape.apply(&resting).is_empty());
    assert!(tape.is_empty());
}

#[test]
fn taker_side_is_the_aggressor() {
    let mut clob = Clob::new();
    let mut tape = TradeTape::new();

    place_limit(&mut clob, Side::Buy, 100, 5);
    let sell = clob.submit(Command::New(NewOrder::limit(Side::Sell, 100, 2)));
    assert_eq!(tape.apply(&sell)[0].taker_side, Side::Sell);

    place_limit(&mut clob, Side::Sell, 105, 5);
    let buy = clob.submit(Command::New(NewOrder::limit(Side::Buy, 105, 2)));
    assert_eq!(tape.apply(&buy)[0].taker_side, Side::Buy);
}

#[test]
fn sweep_prints_each_level_in_execution_order() {
    let mut clob = Clob::new();
    let mut tape = TradeTape::new();
    place_limit(&mut clob, Side::Sell, 102, 5);
    place_limit(&mut clob, Side::Sell, 101, 5);

    let events = clob.submit(Command::New(NewOrder::market(Side::Buy, 7)));
    let out = tape.apply(&events);

    assert_eq!(out.len(), 2);
    assert_eq!((out[0].price, out[0].qty), (101, 5));
    assert_eq!((out[1].price, out[1].qty), (102, 2));
    assert!(out.iter().all(|t| t.seq == clob.current_seq()));
}

#[test]
fn iceberg_prints_full_executed_volume_including_hidden() {
    let mut clob = Clob::new();
    let mut tape = TradeTape::new();
    place_limit(&mut clob, Side::Buy, 100, 4);
    clob.submit(Command::New(NewOrder::iceberg(Side::Sell, 105, 20, 5)));

    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 105, 7)));
    let out = tape.apply(&events);

    let traded: u64 = out.iter().map(|t| t.qty).sum();
    assert_eq!(traded, 7);
    assert!(
        out.iter()
            .all(|t| t.price == 105 && t.taker_side == Side::Buy)
    );
}

#[test]
fn history_accumulates_oldest_first() {
    let mut clob = Clob::new();
    let mut tape = TradeTape::new();
    place_limit(&mut clob, Side::Sell, 101, 10);

    tape.apply(&clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 3))));
    tape.apply(&clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, 2))));

    let qtys: Vec<u64> = tape.recent().map(|t| t.qty).collect();
    assert_eq!(qtys, vec![3, 2]);
    assert_eq!(tape.len(), 2);
    assert_eq!(tape.last().map(|t| t.qty), Some(2));
}

#[test]
fn bounded_tape_evicts_oldest() {
    let mut clob = Clob::new();
    let mut tape = TradeTape::bounded(2);
    place_limit(&mut clob, Side::Sell, 101, 30);

    for qty in [3, 4, 5] {
        tape.apply(&clob.submit(Command::New(NewOrder::limit(Side::Buy, 101, qty))));
    }

    let qtys: Vec<u64> = tape.recent().map(|t| t.qty).collect();
    assert_eq!(qtys, vec![4, 5]);
    assert_eq!(tape.len(), 2);
}

#[test]
fn ioc_partial_fill_prints_only_executed_part() {
    let mut clob = Clob::new();
    let mut tape = TradeTape::new();
    place_limit(&mut clob, Side::Sell, 101, 4);

    let events = clob.submit(Command::New(
        NewOrder::limit(Side::Buy, 101, 10).with_tif(TimeInForce::Ioc),
    ));
    let out = tape.apply(&events);

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].qty, 4);
}

#[test]
fn tape_is_a_faithful_projection_of_the_event_stream() {
    let mut clob = Clob::new();
    let mut tape = TradeTape::new();
    let mut all = Vec::new();

    for command in script() {
        let events = clob.submit(command);
        let out = tape.apply(&events);
        assert_eq!(out, prints(&events));
        all.extend(out);
    }

    assert_eq!(tape.recent().copied().collect::<Vec<_>>(), all);
    assert!(!all.is_empty());
}
