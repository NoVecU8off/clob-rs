use crate::codec::{CodecError, Reader, Writer};
use crate::order::{CancelOrder, Command, ModifyOrder, NewOrder};
use crate::types::{OrderType, Side, TimeInForce};

pub(crate) const MAGIC: [u8; 4] = *b"CLBW";
pub(crate) const FORMAT_VERSION: u16 = 1;
pub(crate) const HEADER_LEN: usize = 14;

const TAG_NEW: u8 = 1;
const TAG_CANCEL: u8 = 2;
const TAG_MODIFY: u8 = 3;

#[derive(Debug)]
pub enum JournalError {
    Io(std::io::Error),
    Codec(CodecError),
    BadMagic,
    UnsupportedVersion(u16),
    UnknownCommand(u8),
    UnknownOrderType(u8),
    UnknownTif(u8),
    CorruptSnapshot,
}

impl From<std::io::Error> for JournalError {
    fn from(err: std::io::Error) -> Self {
        JournalError::Io(err)
    }
}

impl From<CodecError> for JournalError {
    fn from(err: CodecError) -> Self {
        JournalError::Codec(err)
    }
}

pub(crate) fn encode_header(base_seq: u64) -> Vec<u8> {
    let mut w = Writer::with_capacity(HEADER_LEN);
    w.bytes(&MAGIC);
    w.bytes(&FORMAT_VERSION.to_le_bytes());
    w.u64(base_seq);
    w.into_inner()
}

pub(crate) fn decode_header(r: &mut Reader) -> Result<u64, JournalError> {
    if r.bytes(4)? != MAGIC {
        return Err(JournalError::BadMagic);
    }
    let v = r.bytes(2)?;
    let version = u16::from_le_bytes([v[0], v[1]]);
    if version != FORMAT_VERSION {
        return Err(JournalError::UnsupportedVersion(version));
    }
    Ok(r.u64()?)
}

pub(crate) fn encode_command(w: &mut Writer, command: &Command) {
    match command {
        Command::New(order) => {
            w.u8(TAG_NEW);
            let side_bit = matches!(order.side, Side::Sell) as u8;
            w.u8(side_bit | (type_code(order.order_type) << 1) | (tif_code(order.tif) << 4));
            match order.order_type {
                OrderType::Limit => {
                    w.varint(order.price);
                    w.varint(order.qty);
                }
                OrderType::Market => w.varint(order.qty),
                OrderType::Stop { trigger } => {
                    w.varint(trigger);
                    w.varint(order.qty);
                }
                OrderType::StopLimit { trigger } => {
                    w.varint(trigger);
                    w.varint(order.price);
                    w.varint(order.qty);
                }
                OrderType::Iceberg { display } => {
                    w.varint(order.price);
                    w.varint(display);
                    w.varint(order.qty);
                }
            }
        }
        Command::Cancel(cancel) => {
            w.u8(TAG_CANCEL);
            w.varint(cancel.order_id);
        }
        Command::Modify(modify) => {
            w.u8(TAG_MODIFY);
            w.varint(modify.order_id);
            w.varint(modify.price);
            w.varint(modify.qty);
        }
    }
}

pub(crate) fn decode_command(r: &mut Reader) -> Result<Command, JournalError> {
    match r.u8()? {
        TAG_NEW => {
            let flags = r.u8()?;
            let side = if flags & 1 == 1 {
                Side::Sell
            } else {
                Side::Buy
            };
            let tif = decode_tif((flags >> 4) & 0x3)?;
            let (order_type, price, qty) = match (flags >> 1) & 0x7 {
                0 => {
                    let price = r.varint()?;
                    (OrderType::Limit, price, r.varint()?)
                }
                1 => (OrderType::Market, 0, r.varint()?),
                2 => {
                    let trigger = r.varint()?;
                    (OrderType::Stop { trigger }, 0, r.varint()?)
                }
                3 => {
                    let trigger = r.varint()?;
                    let price = r.varint()?;
                    (OrderType::StopLimit { trigger }, price, r.varint()?)
                }
                4 => {
                    let price = r.varint()?;
                    let display = r.varint()?;
                    (OrderType::Iceberg { display }, price, r.varint()?)
                }
                other => return Err(JournalError::UnknownOrderType(other)),
            };
            Ok(Command::New(NewOrder {
                side,
                order_type,
                price,
                qty,
                tif,
            }))
        }
        TAG_CANCEL => Ok(Command::Cancel(CancelOrder {
            order_id: r.varint()?,
        })),
        TAG_MODIFY => Ok(Command::Modify(ModifyOrder {
            order_id: r.varint()?,
            price: r.varint()?,
            qty: r.varint()?,
        })),
        other => Err(JournalError::UnknownCommand(other)),
    }
}

fn type_code(order_type: OrderType) -> u8 {
    match order_type {
        OrderType::Limit => 0,
        OrderType::Market => 1,
        OrderType::Stop { .. } => 2,
        OrderType::StopLimit { .. } => 3,
        OrderType::Iceberg { .. } => 4,
    }
}

pub(crate) fn tif_code(tif: TimeInForce) -> u8 {
    match tif {
        TimeInForce::Gtc => 0,
        TimeInForce::Ioc => 1,
        TimeInForce::Fok => 2,
        TimeInForce::PostOnly => 3,
    }
}

pub(crate) fn decode_tif(code: u8) -> Result<TimeInForce, JournalError> {
    match code {
        0 => Ok(TimeInForce::Gtc),
        1 => Ok(TimeInForce::Ioc),
        2 => Ok(TimeInForce::Fok),
        3 => Ok(TimeInForce::PostOnly),
        other => Err(JournalError::UnknownTif(other)),
    }
}
