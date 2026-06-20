use std::path::Path;

use crate::book::OrderBook;
use crate::clob::Clob;
use crate::journal::JournalError;
use crate::order::Command;
use crate::output::Event;
use crate::wal::{Journal, read_commands};

#[derive(Debug)]
pub struct PersistentClob {
    clob: Clob,
    journal: Journal,
}

impl PersistentClob {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, JournalError> {
        let path = path.as_ref();
        let mut clob = Clob::new();
        let mut scratch = Vec::new();
        for command in read_commands(path)? {
            scratch.clear();
            clob.submit_into(command, &mut scratch);
        }
        let journal = Journal::open(path)?;
        Ok(PersistentClob { clob, journal })
    }

    pub fn submit(&mut self, command: Command) -> Result<Vec<Event>, JournalError> {
        let mut out = Vec::new();
        self.submit_into(command, &mut out)?;
        Ok(out)
    }

    pub fn submit_into(
        &mut self,
        command: Command,
        out: &mut Vec<Event>,
    ) -> Result<(), JournalError> {
        self.journal.append(&command);
        self.journal.commit()?;
        self.clob.submit_into(command, out);
        Ok(())
    }

    pub fn submit_batch(&mut self, commands: &[Command]) -> Result<Vec<Event>, JournalError> {
        for command in commands {
            self.journal.append(command);
        }
        self.journal.commit()?;
        let mut out = Vec::new();
        for command in commands {
            self.clob.submit_into(*command, &mut out);
        }
        Ok(out)
    }

    pub fn flush(&mut self) -> Result<(), JournalError> {
        self.journal.commit()
    }

    pub fn clob(&self) -> &Clob {
        &self.clob
    }

    pub fn book(&self) -> &OrderBook {
        self.clob.book()
    }

    pub fn current_seq(&self) -> u64 {
        self.clob.current_seq()
    }

    pub fn pending_stops(&self) -> usize {
        self.clob.pending_stops()
    }
}
