// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Frame decoders: bytes in, frames out.
//!
//! A [`Decoder`] is fed the bytes of one connection as they arrive, each
//! chunk with its [`Direction`] and time, and gives back [`Frame`]s: the
//! bytes of one message with a name, a one-line summary, named
//! [`Field`]s and a [`Status`] (checksum right or wrong, cut short, not
//! understood). Bytes are never dropped: what a decoder cannot place comes
//! out as an [`Status::Unknown`] frame, so the frames of a stream hold all
//! of its bytes, in order, per direction.
//!
//! Chunks may split messages anywhere; a decoder keeps what it has not
//! finished and decodes the same frames however the bytes were cut. Bytes
//! that wait longer than the decoder's gap without completing a message are
//! given back by [`Decoder::idle`].
//!
//! Nothing here touches a port, a file or a window: the app, the
//! command-line tool and the MCP server feed it from a session's log.
//!
//! Built in: [Modbus RTU](modbus) and [SLCAN](slcan) (CAN adapters that
//! speak the Lawicel ASCII protocol). Other protocols are described in a
//! TOML file ([`custom`]). [`checksum`] has the checksums the
//! send box can append, shared with the command-line tool.

use serde::Serialize;

pub mod checksum;
pub mod custom;
pub mod modbus;
pub mod slcan;

/// The language of names and summaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    /// English.
    #[default]
    En,
    /// Simplified Chinese.
    Zh,
}

/// Which way bytes went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    /// Received from the device.
    Rx,
    /// Sent to the device.
    Tx,
}

impl Direction {
    /// 0 for received, 1 for sent: an index into per-direction state.
    pub fn index(self) -> usize {
        match self {
            Self::Rx => 0,
            Self::Tx => 1,
        }
    }
}

/// How a frame decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    /// A whole message whose check (if it has one) is right.
    Ok,
    /// A whole message whose checksum is wrong; its fields are still shown.
    BadChecksum,
    /// The start of a message whose rest never came.
    Incomplete,
    /// Bytes that are not a message of this protocol.
    Unknown,
}

/// A field's value, for programs (JSON); [`Field::text`] is for people.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Value {
    /// A whole number.
    Int(i64),
    /// A number with a fraction.
    Float(f64),
    /// On or off.
    Bool(bool),
    /// Text, or bytes written as hex.
    Text(String),
    /// Several numbers (registers, coils).
    List(Vec<i64>),
}

/// One named part of a frame.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    /// What it is, in the decoder's language.
    pub name: String,
    /// Its first byte, from the start of the frame.
    pub offset: usize,
    /// How many bytes it takes (0 for a value computed from others).
    pub len: usize,
    /// Its value.
    pub value: Value,
    /// Its value as people read it (`0x0064 (100)`, `ON`, `illegal data address`).
    pub text: String,
}

/// One message, or bytes that are not one.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    /// When its first byte arrived or was sent (microseconds, as the log
    /// keeps them).
    pub timestamp_us: u64,
    /// Which way it went.
    pub direction: Direction,
    /// Its bytes, exactly as they crossed the wire.
    #[serde(serialize_with = "hex_bytes")]
    pub bytes: Vec<u8>,
    /// What kind of message it is (`Read Holding Registers (03)`).
    pub name: String,
    /// The fields that matter, in one line.
    pub summary: String,
    /// Its fields, in byte order.
    pub fields: Vec<Field>,
    /// Whether it decoded whole and checked out.
    pub status: Status,
}

fn hex_bytes<S: serde::Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&hex(bytes))
}

/// Bytes as spaced upper-case hex (`01 03 00 00`).
pub fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push(char::from(HEX_DIGITS[usize::from(b >> 4)]));
        out.push(char::from(HEX_DIGITS[usize::from(b & 0x0f)]));
    }
    out
}

const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";

/// Turns a stream of bytes into frames.
pub trait Decoder: Send {
    /// Takes `bytes` that went `direction` at `timestamp_us` and pushes the
    /// frames they complete onto `out`. Bytes of an unfinished message are
    /// kept for the next call. A chunk that comes more than the decoder's
    /// gap after the previous one in the same direction first ends what was
    /// pending (as [`Decoder::idle`] would).
    fn feed(&mut self, direction: Direction, timestamp_us: u64, bytes: &[u8], out: &mut Vec<Frame>);

    /// Gives back, as frames, bytes that have waited more than the gap by
    /// `now_us` without completing a message. Call it now and then while
    /// the line is quiet, and with `u64::MAX` at the end of the data.
    fn idle(&mut self, now_us: u64, out: &mut Vec<Frame>);
}

/// A protocol SerialBot can decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Protocol {
    /// Its id on the command line and between the app's window and backend.
    pub id: &'static str,
    /// Its name.
    pub name: &'static str,
}

/// The built-in protocols, in the order they are offered.
pub const PROTOCOLS: [Protocol; 2] = [
    Protocol {
        id: "modbus-rtu",
        name: "Modbus RTU",
    },
    Protocol {
        id: "slcan",
        name: "SLCAN",
    },
];

/// A new decoder for the protocol `id` (case ignored), writing names and
/// summaries in `lang`; `None` for an unknown id.
pub fn decoder(id: &str, lang: Lang) -> Option<Box<dyn Decoder>> {
    match id.to_ascii_lowercase().as_str() {
        "modbus-rtu" | "modbus" => Some(Box::new(modbus::ModbusRtu::new(lang))),
        "slcan" => Some(Box::new(slcan::Slcan::new(lang))),
        _ => None,
    }
}

/// The decoder for `spec`: a built-in protocol id (`modbus-rtu`), a
/// protocol file in `dir` (`user:thermostat` or `thermostat`), or the path
/// of a `.toml` file. Gives the protocol's name too. An unknown `spec`
/// fails with the protocols there are; a broken file fails with its
/// mistake.
pub fn find_decoder(
    spec: &str,
    dir: Option<&std::path::Path>,
    lang: Lang,
) -> Result<(String, Box<dyn Decoder>), String> {
    if let Some(d) = decoder(spec, lang) {
        let name = PROTOCOLS
            .iter()
            .find(|p| p.id.eq_ignore_ascii_case(spec))
            .map_or("Modbus RTU", |p| p.name);
        return Ok((name.to_owned(), d));
    }
    let stem = spec.strip_prefix("user:").unwrap_or(spec);
    let in_dir = dir
        .map(|d| d.join(format!("{stem}.toml")))
        .filter(|p| p.is_file());
    let path = in_dir.or_else(|| {
        let p = std::path::PathBuf::from(spec);
        (p.is_file() || spec.to_ascii_lowercase().ends_with(".toml")).then_some(p)
    });
    if let Some(path) = path {
        let custom = custom::load(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        return Ok((custom.name().to_owned(), custom.decoder(lang)));
    }
    let mut known: Vec<String> = PROTOCOLS.iter().map(|p| p.id.to_owned()).collect();
    if let Some(d) = dir {
        known.extend(custom::load_dir(d).into_iter().map(|p| p.id));
    }
    Err(format!(
        "unknown protocol {spec:?}; there are: {}",
        known.join(", ")
    ))
}

/// Decodes a whole recording at once: each `(direction, timestamp_us,
/// bytes)` in order, then whatever is left. Frames come out in the order
/// they ended.
pub fn decode_all<'a>(
    decoder: &mut dyn Decoder,
    chunks: impl IntoIterator<Item = (Direction, u64, &'a [u8])>,
) -> Vec<Frame> {
    let mut out = Vec::new();
    for (direction, timestamp_us, bytes) in chunks {
        decoder.feed(direction, timestamp_us, bytes, &mut out);
    }
    decoder.idle(u64::MAX, &mut out);
    out
}

/// Bytes kept for one direction while a message is incomplete.
#[derive(Debug, Default, Clone)]
pub(crate) struct Pending {
    pub(crate) bytes: Vec<u8>,
    /// When the first pending byte came.
    pub(crate) first_us: u64,
    /// When the last chunk came.
    pub(crate) last_us: u64,
}

impl Pending {
    /// Adds a chunk, remembering when the pending bytes started.
    pub(crate) fn push(&mut self, timestamp_us: u64, bytes: &[u8]) {
        if self.bytes.is_empty() {
            self.first_us = timestamp_us;
        }
        self.bytes.extend_from_slice(bytes);
        self.last_us = timestamp_us;
    }

    /// Whether bytes wait and the line has been quiet for more than `gap_us`
    /// by `now_us`.
    pub(crate) fn stale(&self, now_us: u64, gap_us: u64) -> bool {
        !self.bytes.is_empty() && now_us.saturating_sub(self.last_us) > gap_us
    }

    /// Takes the first `n` pending bytes off, with the time they started.
    /// The rest are taken to have started at the last chunk's time.
    pub(crate) fn take(&mut self, n: usize) -> (u64, Vec<u8>) {
        let started = self.first_us;
        let rest = self.bytes.split_off(n.min(self.bytes.len()));
        let taken = std::mem::replace(&mut self.bytes, rest);
        self.first_us = self.last_us;
        (started, taken)
    }
}

/// A frame of bytes this protocol does not know.
pub(crate) fn unknown(
    direction: Direction,
    timestamp_us: u64,
    bytes: Vec<u8>,
    lang: Lang,
) -> Frame {
    let (name, summary) = match lang {
        Lang::En => ("Unknown".to_owned(), format!("{} bytes", bytes.len())),
        Lang::Zh => ("未识别".to_owned(), format!("{} 字节", bytes.len())),
    };
    Frame {
        timestamp_us,
        direction,
        bytes,
        name,
        summary,
        fields: Vec::new(),
        status: Status::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_is_spaced_upper_case() {
        assert_eq!(hex(&[]), "");
        assert_eq!(hex(&[0x01, 0xab, 0xff]), "01 AB FF");
    }

    #[test]
    fn protocols_have_decoders() {
        for p in PROTOCOLS {
            assert!(decoder(p.id, Lang::En).is_some(), "{}", p.id);
            assert!(decoder(&p.id.to_uppercase(), Lang::Zh).is_some());
        }
        assert!(decoder("modbus", Lang::En).is_some());
        assert!(decoder("mavlink", Lang::En).is_none());
        assert_eq!(Direction::Rx.index(), 0);
        assert_eq!(Direction::Tx.index(), 1);
    }

    #[test]
    fn decoders_are_found_by_id_file_name_or_path() {
        let dir = std::env::temp_dir().join(format!("serialbot-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mine.toml"), custom::EXAMPLE).unwrap();
        std::fs::write(dir.join("broken.toml"), "nope").unwrap();
        let name = |spec: &str| find_decoder(spec, Some(&dir), Lang::En).map(|(n, _)| n);
        assert_eq!(name("SLCAN").unwrap(), "SLCAN");
        assert_eq!(name("modbus").unwrap(), "Modbus RTU");
        assert_eq!(name("user:mine").unwrap(), "Example: thermostat");
        assert_eq!(name("mine").unwrap(), "Example: thermostat");
        let path = dir.join("mine.toml");
        assert_eq!(
            find_decoder(path.to_str().unwrap(), None, Lang::En)
                .map(|(n, _)| n)
                .unwrap(),
            "Example: thermostat"
        );
        assert!(name("broken").unwrap_err().contains("broken.toml"));
        assert!(name("gone.toml").unwrap_err().contains("gone.toml"));
        let err = name("mavlink").unwrap_err();
        assert!(
            err.contains("modbus-rtu, slcan, user:broken, user:mine"),
            "{err}"
        );
        let Err(err) = find_decoder("mavlink", None, Lang::En) else {
            panic!("found");
        };
        assert!(err.ends_with("modbus-rtu, slcan"), "{err}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn frames_serialize_for_programs() {
        let mut frame = unknown(Direction::Tx, 5, vec![1, 2], Lang::En);
        frame.fields.push(Field {
            name: "values".into(),
            offset: 0,
            len: 2,
            value: Value::List(vec![1, 2]),
            text: "1, 2".into(),
        });
        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "timestampUs": 5, "direction": "tx", "bytes": "01 02", "name": "Unknown",
                "summary": "2 bytes", "status": "unknown",
                "fields": [{"name": "values", "offset": 0, "len": 2, "value": [1, 2], "text": "1, 2"}]
            })
        );
        assert_eq!(
            unknown(Direction::Rx, 0, vec![1], Lang::Zh).summary,
            "1 字节"
        );
    }

    #[test]
    fn pending_bytes_keep_their_start_time() {
        let mut p = Pending::default();
        assert!(!p.stale(100, 10));
        p.push(10, b"ab");
        p.push(20, b"cd");
        assert!(!p.stale(25, 10));
        assert!(p.stale(31, 10));
        assert_eq!(p.take(3), (10, b"abc".to_vec()));
        assert_eq!(p.take(9), (20, b"d".to_vec()));
        assert!(p.bytes.is_empty());
    }
}
