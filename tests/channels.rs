mod common;

use clob::{Clob, L2Feed, L3Feed, TradeTape};
use common::script;

const ALL: usize = usize::MAX;

#[test]
fn all_channels_anchor_to_the_command_seq() {
    let mut clob = Clob::new();
    let mut l2 = L2Feed::new();
    let mut l3 = L3Feed::new();
    let mut tape = TradeTape::new();

    let mut prev = 0;
    for command in script() {
        let events = clob.submit(command);
        let seq = clob.current_seq();
        assert_eq!(seq, prev + 1, "seq must advance by one per command");
        prev = seq;

        let u2 = l2.apply(&events, clob.book());
        let u3 = l3.apply(&events, clob.book());
        let prints = tape.apply(&events);

        assert_eq!(u2.seq, seq, "L2 increment seq");
        assert_eq!(u3.seq, seq, "L3 increment seq");
        for print in &prints {
            assert_eq!(print.seq, seq, "trade-tape print seq");
        }
        assert_eq!(clob.l2_snapshot(ALL).seq, seq, "L2 snapshot seq");
        assert_eq!(clob.l3_snapshot().seq, seq, "L3 snapshot seq");
    }
}

#[test]
fn snapshot_and_increments_align_for_a_late_joiner() {
    let mut clob = Clob::new();
    let mut warmup = TradeTape::new();
    for command in script().into_iter().take(6) {
        warmup.apply(&clob.submit(command));
    }

    let base = clob.l2_snapshot(ALL).seq;
    assert_eq!(base, clob.l3_snapshot().seq);

    let mut l2 = L2Feed::from_book(clob.book());
    let mut l3 = L3Feed::from_book(clob.book());
    for command in script().into_iter().skip(6) {
        let events = clob.submit(command);
        let u2 = l2.apply(&events, clob.book());
        let u3 = l3.apply(&events, clob.book());
        assert!(u2.seq > base, "increments must follow the snapshot");
        assert_eq!(u2.seq, u3.seq);
    }
}
