use std::path::PathBuf;

use clob::{Clob, Command, JournalError, NewOrder, PersistentClob, Side, read_commands};

mod common;

use common::{TempFile, capture, script, snap_of};

#[test]
fn checkpoint_rotates_journal_and_recovers_to_plain() {
    let commands = script();
    let tmp = TempFile::new("checkpoint");

    let mut plain = Clob::new();
    let mut plain_events = Vec::new();
    for command in &commands {
        plain_events.extend(plain.submit(*command));
    }
    let plain_state = capture(plain.book(), plain.pending_stops(), plain.current_seq());

    let split = 9;
    let mut live_events = Vec::new();
    {
        let mut persistent = PersistentClob::open(tmp.path()).unwrap();
        for command in &commands[..split] {
            live_events.extend(persistent.submit(*command).unwrap());
        }
        persistent.checkpoint().unwrap();
        assert_eq!(read_commands(tmp.path()).unwrap().len(), 0);
        for command in &commands[split..] {
            live_events.extend(persistent.submit(*command).unwrap());
        }
        assert_eq!(
            read_commands(tmp.path()).unwrap().len(),
            commands.len() - split
        );
        assert_eq!(
            capture(
                persistent.book(),
                persistent.pending_stops(),
                persistent.current_seq()
            ),
            plain_state
        );
    }
    assert_eq!(live_events, plain_events);

    let mut recovered = PersistentClob::open(tmp.path()).unwrap();
    assert_eq!(
        capture(
            recovered.book(),
            recovered.pending_stops(),
            recovered.current_seq()
        ),
        plain_state
    );

    let probe = Command::New(NewOrder::limit(Side::Buy, 105, 4));
    assert_eq!(recovered.submit(probe).unwrap(), plain.submit(probe));
}

#[test]
fn multiple_checkpoints_recover_to_plain() {
    let commands = script();
    let tmp = TempFile::new("multi");

    let mut plain = Clob::new();
    for command in &commands {
        plain.submit(*command);
    }
    let plain_state = capture(plain.book(), plain.pending_stops(), plain.current_seq());

    {
        let mut persistent = PersistentClob::open(tmp.path()).unwrap();
        for (i, command) in commands.iter().enumerate() {
            persistent.submit(*command).unwrap();
            if i == 3 || i == 7 {
                persistent.checkpoint().unwrap();
            }
        }
    }

    let recovered = PersistentClob::open(tmp.path()).unwrap();
    assert_eq!(
        capture(
            recovered.book(),
            recovered.pending_stops(),
            recovered.current_seq()
        ),
        plain_state
    );
}

#[test]
fn recovery_dedups_stale_journal_after_interrupted_checkpoint() {
    let commands = script();
    let tmp = TempFile::new("dedup");

    let mut plain = Clob::new();
    for command in &commands {
        plain.submit(*command);
    }
    let plain_state = capture(plain.book(), plain.pending_stops(), plain.current_seq());

    let mut stale = tmp.path().to_path_buf().into_os_string();
    stale.push(".stale");
    let stale = PathBuf::from(stale);

    {
        let mut persistent = PersistentClob::open(tmp.path()).unwrap();
        for command in &commands {
            persistent.submit(*command).unwrap();
        }
        std::fs::copy(tmp.path(), &stale).unwrap();
        persistent.checkpoint().unwrap();
    }

    std::fs::copy(&stale, tmp.path()).unwrap();
    let _ = std::fs::remove_file(&stale);

    let recovered = PersistentClob::open(tmp.path()).unwrap();
    assert_eq!(
        capture(
            recovered.book(),
            recovered.pending_stops(),
            recovered.current_seq()
        ),
        plain_state
    );
}

#[test]
fn corrupt_snapshot_is_rejected() {
    let commands = script();
    let tmp = TempFile::new("corrupt");
    {
        let mut persistent = PersistentClob::open(tmp.path()).unwrap();
        for command in &commands {
            persistent.submit(*command).unwrap();
        }
        persistent.checkpoint().unwrap();
    }

    let snap = snap_of(tmp.path());
    let mut bytes = std::fs::read(&snap).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xFF;
    std::fs::write(&snap, &bytes).unwrap();

    assert!(matches!(
        PersistentClob::open(tmp.path()),
        Err(JournalError::CorruptSnapshot)
    ));
}

#[test]
fn checkpoint_on_empty_state_recovers_empty() {
    let tmp = TempFile::new("empty_ckpt");
    {
        let mut persistent = PersistentClob::open(tmp.path()).unwrap();
        persistent.checkpoint().unwrap();
    }
    let recovered = PersistentClob::open(tmp.path()).unwrap();
    assert_eq!(recovered.current_seq(), 0);
    assert!(recovered.book().is_empty());
    assert_eq!(recovered.pending_stops(), 0);
}
