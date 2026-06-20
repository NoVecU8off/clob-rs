use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::codec::{Reader, Writer, crc32};
use crate::journal::JournalError;
use crate::journal::{decode_command, decode_header, encode_command, encode_header};
use crate::order::Command;

#[derive(Debug)]
pub struct Journal {
    file: File,
    path: PathBuf,
    block: Writer,
    pending: u64,
}

impl Journal {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, JournalError> {
        Journal::open_base(path, 0)
    }

    pub(crate) fn open_base<P: AsRef<Path>>(path: P, base_seq: u64) -> Result<Self, JournalError> {
        let path = path.as_ref().to_path_buf();
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        if file.metadata()?.len() == 0 {
            file.write_all(&encode_header(base_seq))?;
            file.sync_all()?;
        }
        Ok(Journal {
            file,
            path,
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

    pub(crate) fn rotate(&mut self, base_seq: u64) -> Result<(), JournalError> {
        let tmp = tmp_path(&self.path);
        {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp)?;
            file.write_all(&encode_header(base_seq))?;
            file.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.block.clear();
        self.pending = 0;
        Ok(())
    }

    pub fn pending(&self) -> u64 {
        self.pending
    }
}

pub(crate) fn read_segment<P: AsRef<Path>>(path: P) -> Result<(u64, Vec<Command>), JournalError> {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok((0, Vec::new())),
        Err(err) => return Err(err.into()),
    };
    if data.is_empty() {
        return Ok((0, Vec::new()));
    }
    let mut reader = Reader::new(&data);
    let base_seq = decode_header(&mut reader)?;
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
    Ok((base_seq, commands))
}

pub fn read_commands<P: AsRef<Path>>(path: P) -> Result<Vec<Command>, JournalError> {
    Ok(read_segment(path)?.1)
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}
