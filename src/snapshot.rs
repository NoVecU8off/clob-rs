use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::book::{RestingEntry, RestingOrder};
use crate::clob::Clob;
use crate::codec::{Reader, Writer, crc32};
use crate::journal::{JournalError, decode_stp, decode_tif, stp_code, tif_code};
use crate::stops::PendingStop;
use crate::types::{AccountId, OrderId, OrderType, Price, SeqNum, Side};

const SNAP_MAGIC: [u8; 4] = *b"CLBS";
const SNAP_VERSION: u16 = 3;
const MIN_LEN: usize = 4 + 2 + 4;

pub(crate) struct SnapshotState {
    pub(crate) seq: SeqNum,
    pub(crate) next_order_id: OrderId,
    pub(crate) last_trade_price: Option<Price>,
    pub(crate) orders: Vec<RestingEntry>,
    pub(crate) stops: Vec<PendingStop>,
    pub(crate) account_net: Vec<(AccountId, i128)>,
}

pub(crate) fn write(path: &Path, clob: &Clob) -> Result<(), JournalError> {
    let bytes = encode(&clob.capture());
    let tmp = tmp_path(path);
    {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

pub(crate) fn read(path: &Path) -> Result<Option<Clob>, JournalError> {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    Ok(Some(Clob::from_snapshot(decode(&data)?)))
}

fn encode(state: &SnapshotState) -> Vec<u8> {
    let mut w = Writer::new();
    w.bytes(&SNAP_MAGIC);
    w.bytes(&SNAP_VERSION.to_le_bytes());
    w.varint(state.seq);
    w.varint(state.next_order_id);
    match state.last_trade_price {
        Some(price) => {
            w.u8(1);
            w.varint(price);
        }
        None => w.u8(0),
    }
    w.varint(state.orders.len() as u64);
    for (side, order, reserve) in &state.orders {
        w.u8(side_code(*side));
        w.varint(order.id);
        w.varint(order.seq);
        w.varint(order.price);
        w.varint(order.qty);
        w.varint(order.timestamp);
        w.varint(order.owner);
        match reserve {
            Some((display, hidden)) => {
                w.u8(1);
                w.varint(*display);
                w.varint(*hidden);
            }
            None => w.u8(0),
        }
    }
    w.varint(state.stops.len() as u64);
    for stop in &state.stops {
        w.varint(stop.id);
        w.u8(side_code(stop.side));
        w.varint(stop.trigger);
        w.u8(matches!(stop.activates_to, OrderType::Limit) as u8);
        w.varint(stop.limit_price);
        w.varint(stop.qty);
        w.u8(tif_code(stop.tif));
        w.varint(stop.owner);
        w.u8(stp_code(stop.stp));
    }
    w.varint(state.account_net.len() as u64);
    for (owner, net) in &state.account_net {
        w.varint(*owner);
        w.varint128(zigzag(*net));
    }
    let crc = crc32(w.as_slice());
    w.u32(crc);
    w.into_inner()
}

fn decode(data: &[u8]) -> Result<SnapshotState, JournalError> {
    if data.len() < MIN_LEN {
        return Err(JournalError::CorruptSnapshot);
    }
    let (body, crc_bytes) = data.split_at(data.len() - 4);
    let crc = u32::from_le_bytes([crc_bytes[0], crc_bytes[1], crc_bytes[2], crc_bytes[3]]);
    if crc32(body) != crc {
        return Err(JournalError::CorruptSnapshot);
    }
    let mut r = Reader::new(body);
    if r.bytes(4)? != SNAP_MAGIC {
        return Err(JournalError::BadMagic);
    }
    let v = r.bytes(2)?;
    let version = u16::from_le_bytes([v[0], v[1]]);
    if version != SNAP_VERSION {
        return Err(JournalError::UnsupportedVersion(version));
    }
    let seq = r.varint()?;
    let next_order_id = r.varint()?;
    let last_trade_price = if r.u8()? == 1 {
        Some(r.varint()?)
    } else {
        None
    };
    let order_count = r.varint()?;
    let mut orders = Vec::with_capacity(order_count as usize);
    for _ in 0..order_count {
        let side = decode_side(r.u8()?);
        let order = RestingOrder {
            id: r.varint()?,
            seq: r.varint()?,
            price: r.varint()?,
            qty: r.varint()?,
            timestamp: r.varint()?,
            owner: r.varint()?,
        };
        let reserve = if r.u8()? == 1 {
            Some((r.varint()?, r.varint()?))
        } else {
            None
        };
        orders.push((side, order, reserve));
    }
    let stop_count = r.varint()?;
    let mut stops = Vec::with_capacity(stop_count as usize);
    for _ in 0..stop_count {
        let id = r.varint()?;
        let side = decode_side(r.u8()?);
        let trigger = r.varint()?;
        let activates_to = if r.u8()? == 1 {
            OrderType::Limit
        } else {
            OrderType::Market
        };
        let limit_price = r.varint()?;
        let qty = r.varint()?;
        let tif = decode_tif(r.u8()?)?;
        let owner = r.varint()?;
        let stp = decode_stp(r.u8()?);
        stops.push(PendingStop {
            id,
            side,
            trigger,
            activates_to,
            limit_price,
            qty,
            tif,
            owner,
            stp,
        });
    }
    let account_count = r.varint()?;
    let mut account_net = Vec::with_capacity(account_count as usize);
    for _ in 0..account_count {
        let owner = r.varint()?;
        let net = unzigzag(r.varint128()?);
        account_net.push((owner, net));
    }
    Ok(SnapshotState {
        seq,
        next_order_id,
        last_trade_price,
        orders,
        stops,
        account_net,
    })
}

fn zigzag(n: i128) -> u128 {
    ((n << 1) ^ (n >> 127)) as u128
}

fn unzigzag(z: u128) -> i128 {
    (z >> 1) as i128 ^ -((z & 1) as i128)
}

fn side_code(side: Side) -> u8 {
    matches!(side, Side::Sell) as u8
}

fn decode_side(code: u8) -> Side {
    if code == 1 { Side::Sell } else { Side::Buy }
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".tmp");
    PathBuf::from(name)
}
