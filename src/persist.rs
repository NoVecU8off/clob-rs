use std::path::{Path, PathBuf};

use crate::book::OrderBook;
use crate::clob::Clob;
use crate::journal::JournalError;
use crate::order::Command;
use crate::output::Event;
use crate::risk::RiskConfig;
use crate::snapshot;
use crate::wal::{Journal, read_segment};

#[derive(Debug)]
pub struct PersistentClob {
    clob: Clob,
    journal: Journal,
    snapshot_path: PathBuf,
}

impl PersistentClob {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, JournalError> {
        Self::open_with_risk(path, RiskConfig::default())
    }

    pub fn open_with_risk<P: AsRef<Path>>(path: P, risk: RiskConfig) -> Result<Self, JournalError> {
        let path = path.as_ref();
        let snapshot_path = snapshot_path(path);
        let (mut clob, applied) = match snapshot::read(&snapshot_path)? {
            Some(clob) => {
                let applied = clob.current_seq();
                (clob, applied)
            }
            None => (Clob::new(), 0),
        };
        clob.set_risk(risk);
        let (base_seq, commands) = read_segment(path)?;
        let mut scratch = Vec::new();
        for (offset, command) in commands.into_iter().enumerate() {
            let seq = base_seq + offset as u64 + 1;
            if seq > applied {
                scratch.clear();
                clob.submit_into(command, &mut scratch);
            }
        }
        let journal = Journal::open_base(path, applied)?;
        Ok(PersistentClob {
            clob,
            journal,
            snapshot_path,
        })
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

    pub fn checkpoint(&mut self) -> Result<(), JournalError> {
        self.journal.commit()?;
        let base_seq = self.clob.current_seq();
        snapshot::write(&self.snapshot_path, &self.clob)?;
        self.journal.rotate(base_seq)?;
        Ok(())
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

fn snapshot_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".snap");
    PathBuf::from(name)
}
