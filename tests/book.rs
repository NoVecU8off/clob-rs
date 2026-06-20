mod common;

use clob::{CancelOrder, Clob, Command, NewOrder, Side};

use common::place_limit;

#[test]
fn bids_depth_is_aggregated_and_best_first() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Buy, 100, 5);
    place_limit(&mut clob, Side::Buy, 100, 3);
    place_limit(&mut clob, Side::Buy, 99, 7);
    place_limit(&mut clob, Side::Buy, 101, 2);

    assert_eq!(
        clob.book().depth(Side::Buy, 3),
        vec![(101, 2), (100, 8), (99, 7)]
    );
}

#[test]
fn asks_depth_is_ascending() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Sell, 105, 1);
    place_limit(&mut clob, Side::Sell, 103, 4);
    place_limit(&mut clob, Side::Sell, 104, 2);

    assert_eq!(
        clob.book().depth(Side::Sell, 5),
        vec![(103, 4), (104, 2), (105, 1)]
    );
}

#[test]
fn best_prices_and_spread() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Buy, 100, 5);
    place_limit(&mut clob, Side::Sell, 103, 5);

    assert_eq!(clob.book().best_bid(), Some(100));
    assert_eq!(clob.book().best_ask(), Some(103));
    assert_eq!(clob.book().spread(), Some(3));
}

#[test]
fn len_counts_resting_orders() {
    let mut clob = Clob::new();
    assert!(clob.book().is_empty());

    place_limit(&mut clob, Side::Buy, 100, 5);
    place_limit(&mut clob, Side::Buy, 99, 5);
    assert_eq!(clob.book().len(), 2);
}

#[test]
fn available_qty_sums_crossable_liquidity() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Sell, 101, 5);
    place_limit(&mut clob, Side::Sell, 102, 4);
    place_limit(&mut clob, Side::Sell, 103, 3);

    assert_eq!(clob.book().available_qty(Side::Buy, Some(102)), 9);
    assert_eq!(clob.book().available_qty(Side::Buy, None), 12);
}

#[test]
fn same_commands_produce_same_events() {
    let commands = || {
        vec![
            Command::New(NewOrder::limit(Side::Sell, 102, 5)),
            Command::New(NewOrder::limit(Side::Sell, 101, 5)),
            Command::New(NewOrder::limit(Side::Buy, 101, 8)),
            Command::New(NewOrder::limit(Side::Buy, 100, 4)),
            Command::Cancel(CancelOrder { order_id: 4 }),
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
