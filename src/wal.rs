use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use crate::codec::{Reader, Writer, crc32};
use crate::journal::JournalError;
use crate::journal::{decode_command, decode_header, encode_command, encode_header};
use crate::order::Command;

#[derive(Debug)]
pub struct Journal {
    file: File,
    block: Writer,
    pending: u64,
}

impl Journal {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, JournalError> {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        if file.metadata()?.len() == 0 {
            file.write_all(&encode_header(0))?;
            file.sync_all()?;
        }
        Ok(Journal {
            file,
            block: Writer::new(),
            pending: 0,
        })
    }

    pub fn append(&mut self, command: &Command) {
        encode_command(&mut self.block, command);
        self.pending += 1;
    }

    pub fn commit(&mut self) -> Result<(), JournalError> {
        if self.block.is_empty() {
            return Ok(());
        }
        let mut body = Writer::new();
        body.varint(self.pending);
        body.bytes(self.block.as_slice());
        let crc = crc32(body.as_slice());
        let block_len = (body.len() + 4) as u32;
        let mut frame = Writer::with_capacity(4 + block_len as usize);
        frame.u32(block_len);
        frame.bytes(body.as_slice());
        frame.u32(crc);
        self.file.write_all(frame.as_slice())?;
        self.file.sync_all()?;
        self.block.clear();
        self.pending = 0;
        Ok(())
    }

    pub fn pending(&self) -> u64 {
        self.pending
    }
}

pub fn read_commands<P: AsRef<Path>>(path: P) -> Result<Vec<Command>, JournalError> {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let mut reader = Reader::new(&data);
    decode_header(&mut reader)?;
    let mut commands = Vec::new();
    while reader.remaining() >= 4 {
        let block_len = reader.u32()? as usize;
        let Ok(block) = reader.bytes(block_len) else {
            break;
        };
        if block.len() < 4 {
            break;
        }
        let (body, crc) = block.split_at(block.len() - 4);
        if crc32(body) != u32::from_le_bytes([crc[0], crc[1], crc[2], crc[3]]) {
            break;
        }
        let mut body_reader = Reader::new(body);
        let count = body_reader.varint()?;
        for _ in 0..count {
            commands.push(decode_command(&mut body_reader)?);
        }
    }
    Ok(commands)
}
