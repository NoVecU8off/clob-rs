mod common;

use clob::{CancelOrder, Clob, Command, Event, NewOrder, RejectReason, Side, TimeInForce};

use common::{accepted_id, place_limit};

fn triggered_ids(events: &[Event]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Triggered { order_id, .. } => Some(*order_id),
            _ => None,
        })
        .collect()
}

fn taker_trades(events: &[Event], taker: u64) -> Vec<(u64, u64)> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Trade {
                taker_order_id,
                price,
                qty,
                ..
            } if *taker_order_id == taker => Some((*price, *qty)),
            _ => None,
        })
        .collect()
}

#[test]
fn stop_parks_and_stays_hidden_until_triggered() {
    let mut clob = Clob::new();
    let events = clob.submit(Command::New(NewOrder::stop(Side::Buy, 200, 5)));
    let id = accepted_id(&events);
    assert!(triggered_ids(&events).is_empty());
    assert_eq!(clob.pending_stops(), 1);
    assert_eq!(clob.book().len(), 0);
    assert!(!clob.book().contains(id));
    assert!(clob.book().depth(Side::Buy, 5).is_empty());
}

#[test]
fn buy_stop_triggers_on_trade_at_or_above_trigger() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Sell, 100, 10);
    let stop = accepted_id(&clob.submit(Command::New(NewOrder::stop(Side::Buy, 100, 4))));
    assert_eq!(clob.pending_stops(), 1);

    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 1)));
    assert_eq!(triggered_ids(&events), vec![stop]);
    assert_eq!(taker_trades(&events, stop), vec![(100, 4)]);
    assert_eq!(clob.pending_stops(), 0);
}

#[test]
fn sell_stop_triggers_on_trade_at_or_below_trigger() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Buy, 100, 10);
    let stop = accepted_id(&clob.submit(Command::New(NewOrder::stop(Side::Sell, 100, 4))));

    let events = clob.submit(Command::New(NewOrder::limit(Side::Sell, 100, 1)));
    assert_eq!(triggered_ids(&events), vec![stop]);
    assert_eq!(taker_trades(&events, stop), vec![(100, 4)]);
    assert_eq!(clob.pending_stops(), 0);
}

#[test]
fn stop_does_not_trigger_when_price_short_of_trigger() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Sell, 100, 10);
    clob.submit(Command::New(NewOrder::stop(Side::Buy, 110, 4)));

    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 1)));
    assert!(triggered_ids(&events).is_empty());
    assert_eq!(clob.pending_stops(), 1);
}

#[test]
fn stop_triggers_immediately_if_market_already_past_trigger() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Sell, 100, 10);
    clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 1)));

    let events = clob.submit(Command::New(NewOrder::stop(Side::Buy, 100, 4)));
    let stop = accepted_id(&events);
    assert_eq!(triggered_ids(&events), vec![stop]);
    assert_eq!(taker_trades(&events, stop), vec![(100, 4)]);
    assert_eq!(clob.pending_stops(), 0);
}

#[test]
fn stop_limit_rests_when_limit_does_not_cross() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Sell, 100, 10);
    let stop = accepted_id(&clob.submit(Command::New(NewOrder::stop_limit(Side::Buy, 100, 99, 4))));

    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 1)));
    assert_eq!(triggered_ids(&events), vec![stop]);
    assert!(taker_trades(&events, stop).is_empty());
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Resting { order_id, price, qty, .. }
            if *order_id == stop && *price == 99 && *qty == 4
    )));
    assert_eq!(clob.book().best_bid(), Some(99));
    assert!(clob.book().contains(stop));
    assert_eq!(clob.pending_stops(), 0);
}

#[test]
fn stop_limit_fills_when_limit_crosses() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Sell, 100, 10);
    let stop =
        accepted_id(&clob.submit(Command::New(NewOrder::stop_limit(Side::Buy, 100, 101, 4))));

    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 1)));
    assert_eq!(triggered_ids(&events), vec![stop]);
    assert_eq!(taker_trades(&events, stop), vec![(100, 4)]);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Filled { order_id, .. } if *order_id == stop))
    );
}

#[test]
fn stop_activation_cascades_into_further_stops() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Sell, 100, 3);
    place_limit(&mut clob, Side::Sell, 110, 10);
    let a = accepted_id(&clob.submit(Command::New(NewOrder::stop(Side::Buy, 100, 5))));
    let b = accepted_id(&clob.submit(Command::New(NewOrder::stop(Side::Buy, 110, 2))));
    assert_eq!(clob.pending_stops(), 2);

    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 1)));
    assert_eq!(triggered_ids(&events), vec![a, b]);
    assert_eq!(taker_trades(&events, a), vec![(100, 2), (110, 3)]);
    assert_eq!(taker_trades(&events, b), vec![(110, 2)]);
    assert_eq!(clob.pending_stops(), 0);
}

#[test]
fn pending_stop_can_be_canceled() {
    let mut clob = Clob::new();
    let stop = accepted_id(&clob.submit(Command::New(NewOrder::stop(Side::Buy, 200, 5))));
    assert_eq!(clob.pending_stops(), 1);

    let events = clob.submit(Command::Cancel(CancelOrder { order_id: stop }));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Canceled { order_id, .. } if *order_id == stop))
    );
    assert_eq!(clob.pending_stops(), 0);
}

#[test]
fn stop_limit_post_only_rejected_on_trigger_if_it_would_cross() {
    let mut clob = Clob::new();
    place_limit(&mut clob, Side::Sell, 100, 10);
    let stop = accepted_id(&clob.submit(Command::New(
        NewOrder::stop_limit(Side::Buy, 100, 105, 4).with_tif(TimeInForce::PostOnly),
    )));

    let events = clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 1)));
    assert_eq!(triggered_ids(&events), vec![stop]);
    assert!(events.iter().any(|e| matches!(
        e,
        Event::Rejected {
            reason: RejectReason::WouldCross,
            ..
        }
    )));
    assert_eq!(clob.pending_stops(), 0);
}

#[test]
fn zero_trigger_is_rejected() {
    let mut clob = Clob::new();
    let events = clob.submit(Command::New(NewOrder::stop(Side::Buy, 0, 5)));
    assert!(matches!(
        events.as_slice(),
        [Event::Rejected {
            reason: RejectReason::InvalidPrice,
            ..
        }]
    ));
    assert_eq!(clob.pending_stops(), 0);
}

#[test]
fn stop_limit_zero_limit_is_rejected() {
    let mut clob = Clob::new();
    let events = clob.submit(Command::New(NewOrder::stop_limit(Side::Buy, 100, 0, 5)));
    assert!(matches!(
        events.as_slice(),
        [Event::Rejected {
            reason: RejectReason::InvalidPrice,
            ..
        }]
    ));
}

#[test]
fn stop_cascade_is_deterministic() {
    fn script(clob: &mut Clob) -> Vec<Event> {
        let mut out = Vec::new();
        out.extend(clob.submit(Command::New(NewOrder::limit(Side::Sell, 100, 3))));
        out.extend(clob.submit(Command::New(NewOrder::limit(Side::Sell, 110, 10))));
        out.extend(clob.submit(Command::New(NewOrder::stop(Side::Buy, 100, 5))));
        out.extend(clob.submit(Command::New(NewOrder::stop(Side::Buy, 110, 2))));
        out.extend(clob.submit(Command::New(NewOrder::limit(Side::Buy, 100, 1))));
        out
    }
    let mut a = Clob::new();
    let mut b = Clob::new();
    assert_eq!(script(&mut a), script(&mut b));
}
