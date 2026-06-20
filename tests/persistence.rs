use std::fs::OpenOptions;

use clob::{
    CancelOrder, Clob, Command, Journal, JournalError, ModifyOrder, NewOrder, PersistentClob, Side,
    TimeInForce, read_commands,
};

mod common;

use common::{TempFile, capture, script};

#[test]
fn command_round_trip_covers_all_shapes() {
    let tmp = TempFile::new("roundtrip");
    let commands = vec![
        Command::New(NewOrder::limit(Side::Buy, 101, 5)),
        Command::New(NewOrder::limit(Side::Sell, 202, 7).with_tif(TimeInForce::PostOnly)),
        Command::New(NewOrder::limit(Side::Buy, 99, 3).with_tif(TimeInForce::Fok)),
        Command::New(NewOrder::market(Side::Buy, 8)),
        Command::New(NewOrder::market(Side::Sell, 9)),
        Command::New(NewOrder::stop(Side::Buy, 300, 4)),
        Command::New(NewOrder::stop(Side::Sell, 50, 6)),
        Command::New(NewOrder::stop_limit(Side::Buy, 300, 305, 4)),
        Command::New(NewOrder::stop_limit(Side::Sell, 50, 45, 6)),
        Command::New(NewOrder::iceberg(Side::Buy, 100, 1000, 10)),
        Command::New(NewOrder::iceberg(Side::Sell, 200, 2000, 20)),
        Command::Cancel(CancelOrder { order_id: 7 }),
        Command::Modify(ModifyOrder::new(42, 123, 456)),
    ];

    {
        let mut journal = Journal::open(tmp.path()).unwrap();
        for command in &commands {
            journal.append(command);
        }
        journal.commit().unwrap();
    }

    assert_eq!(read_commands(tmp.path()).unwrap(), commands);
}

#[test]
fn replay_reconstructs_state_and_matches_plain_clob() {
    let commands = script();
    let tmp = TempFile::new("replay");

    let mut plain = Clob::new();
    let mut plain_events = Vec::new();
    for command in &commands {
        plain_events.extend(plain.submit(*command));
    }
    let plain_state = capture(plain.book(), plain.pending_stops(), plain.current_seq());

    let mut live_events = Vec::new();
    {
        let mut persistent = PersistentClob::open(tmp.path()).unwrap();
        for command in &commands {
            live_events.extend(persistent.submit(*command).unwrap());
        }
        let live_state = capture(
            persistent.book(),
            persistent.pending_stops(),
            persistent.current_seq(),
        );
        assert_eq!(live_state, plain_state);
    }
    assert_eq!(live_events, plain_events);

    let recovered = PersistentClob::open(tmp.path()).unwrap();
    let recovered_state = capture(
        recovered.book(),
        recovered.pending_stops(),
        recovered.current_seq(),
    );
    assert_eq!(recovered_state, plain_state);
}

#[test]
fn rejected_commands_consume_seq_on_replay() {
    let tmp = TempFile::new("rejseq");
    {
        let mut persistent = PersistentClob::open(tmp.path()).unwrap();
        persistent
            .submit(Command::New(NewOrder::limit(Side::Buy, 100, 0)))
            .unwrap();
        persistent
            .submit(Command::New(NewOrder::limit(Side::Buy, 100, 5)))
            .unwrap();
    }

    let recovered = PersistentClob::open(tmp.path()).unwrap();
    assert_eq!(recovered.current_seq(), 2);
    assert_eq!(recovered.book().depth(Side::Buy, 4), vec![(100, 5)]);
    assert!(recovered.book().contains(1));
}

#[test]
fn torn_tail_block_is_ignored_on_recovery() {
    let tmp = TempFile::new("torntail");
    let intact = {
        let mut persistent = PersistentClob::open(tmp.path()).unwrap();
        persistent
            .submit(Command::New(NewOrder::limit(Side::Sell, 100, 5)))
            .unwrap();
        persistent
            .submit(Command::New(NewOrder::limit(Side::Buy, 99, 4)))
            .unwrap();
        let state = capture(
            persistent.book(),
            persistent.pending_stops(),
            persistent.current_seq(),
        );
        persistent
            .submit(Command::New(NewOrder::limit(Side::Buy, 98, 3)))
            .unwrap();
        state
    };

    let len = std::fs::metadata(tmp.path()).unwrap().len();
    {
        let file = OpenOptions::new().write(true).open(tmp.path()).unwrap();
        file.set_len(len - 1).unwrap();
    }

    let recovered = PersistentClob::open(tmp.path()).unwrap();
    let recovered_state = capture(
        recovered.book(),
        recovered.pending_stops(),
        recovered.current_seq(),
    );
    assert_eq!(recovered_state, intact);
}

#[test]
fn bad_magic_is_rejected() {
    let tmp = TempFile::new("badmagic");
    std::fs::write(tmp.path(), b"XXXX\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00").unwrap();
    assert!(matches!(
        read_commands(tmp.path()),
        Err(JournalError::BadMagic)
    ));
}

#[test]
fn unsupported_version_is_rejected() {
    let tmp = TempFile::new("badversion");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"CLBW");
    bytes.extend_from_slice(&99u16.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    std::fs::write(tmp.path(), &bytes).unwrap();
    assert!(matches!(
        read_commands(tmp.path()),
        Err(JournalError::UnsupportedVersion(99))
    ));
}

#[test]
fn missing_journal_recovers_empty() {
    let tmp = TempFile::new("missing");
    let persistent = PersistentClob::open(tmp.path()).unwrap();
    assert_eq!(persistent.current_seq(), 0);
    assert!(persistent.book().is_empty());
    assert_eq!(persistent.pending_stops(), 0);
}

#[test]
fn batch_commit_matches_per_command() {
    let commands = script();
    let tmp_a = TempFile::new("batch_a");
    let tmp_b = TempFile::new("batch_b");

    let mut per_command = PersistentClob::open(tmp_a.path()).unwrap();
    let mut events_a = Vec::new();
    for command in &commands {
        events_a.extend(per_command.submit(*command).unwrap());
    }

    let mut batched = PersistentClob::open(tmp_b.path()).unwrap();
    let events_b = batched.submit_batch(&commands).unwrap();

    assert_eq!(events_a, events_b);
    assert_eq!(
        capture(
            per_command.book(),
            per_command.pending_stops(),
            per_command.current_seq()
        ),
        capture(
            batched.book(),
            batched.pending_stops(),
            batched.current_seq()
        )
    );

    drop(per_command);
    drop(batched);
    let recovered_a = PersistentClob::open(tmp_a.path()).unwrap();
    let recovered_b = PersistentClob::open(tmp_b.path()).unwrap();
    assert_eq!(
        capture(
            recovered_a.book(),
            recovered_a.pending_stops(),
            recovered_a.current_seq()
        ),
        capture(
            recovered_b.book(),
            recovered_b.pending_stops(),
            recovered_b.current_seq()
        )
    );
}
