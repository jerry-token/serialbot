// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Modbus RTU: slave address, function code, data, CRC-16 (low byte first).
//!
//! RTU marks the end of a message with 3.5 character times of silence, but
//! USB serial adapters hand bytes over in batches, so silence is a poor
//! guide. Messages are found by length instead: the function code and the
//! direction say how long a request or a response is (some carry a byte
//! count), and the CRC confirms it. Bytes sent are taken to be requests and
//! bytes received responses first; requests among received bytes (a
//! listener on someone else's bus) are recognised by their CRC.
//!
//! Function codes 01–08, 11, 15–17, 22 and 23 are decoded field by field,
//! and exception responses name their exception. A response to a read is
//! paired with the request before it, so its registers are named by
//! address. Other function codes come out whole after a quiet gap, decoded
//! as far as slave, function and CRC.

use std::time::Duration;

use crate::checksum::crc16_modbus;
use crate::{Decoder, Direction, Field, Frame, Lang, Pending, Status, Value, unknown};

/// The longest RTU message: 256 bytes.
const MAX_LEN: usize = 256;

/// How many register or coil values a summary lists before "…".
const SUMMARY_VALUES: usize = 8;

/// A Modbus RTU decoder.
#[derive(Debug)]
pub struct ModbusRtu {
    lang: Lang,
    gap_us: u64,
    pending: [Pending; 2],
    /// The last read or write request, to name a response's values.
    request: Option<Request>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Request {
    slave: u8,
    function: u8,
    address: u16,
    quantity: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Request,
    Response,
    Exception,
}

/// A possible message at the start of the buffer.
#[derive(Debug, Clone, Copy)]
struct Candidate {
    kind: Kind,
    /// Its length; `None` while the byte that tells it has not arrived.
    len: Option<usize>,
}

enum Split {
    /// A whole message with a good CRC, this long, of this kind.
    Found(usize, Kind),
    /// More bytes may complete a message.
    Wait,
    /// The first byte starts no message.
    Skip,
}

impl ModbusRtu {
    /// Bytes waiting longer than this without completing a message are
    /// given back: 50 ms, longer than a USB adapter's batching and far
    /// shorter than a master's polling interval.
    pub const DEFAULT_GAP: Duration = Duration::from_millis(50);

    /// A decoder writing names and summaries in `lang`.
    pub fn new(lang: Lang) -> Self {
        Self {
            lang,
            gap_us: duration_us(Self::DEFAULT_GAP),
            pending: [Pending::default(), Pending::default()],
            request: None,
        }
    }

    /// The same decoder with another gap (see [`Self::DEFAULT_GAP`]).
    #[must_use]
    pub fn with_gap(mut self, gap: Duration) -> Self {
        self.gap_us = duration_us(gap);
        self
    }

    /// Decodes what is pending in `direction`; with `flush`, also what can
    /// no longer grow.
    fn drain(&mut self, direction: Direction, flush: bool, out: &mut Vec<Frame>) {
        let d = direction.index();
        let mut junk: Vec<u8> = Vec::new();
        let mut junk_at = 0;
        loop {
            if self.pending[d].bytes.is_empty() {
                break;
            }
            match split(&self.pending[d].bytes, direction) {
                Split::Found(len, kind) => {
                    if !junk.is_empty() {
                        out.push(unknown(
                            direction,
                            junk_at,
                            std::mem::take(&mut junk),
                            self.lang,
                        ));
                    }
                    let (at, bytes) = self.pending[d].take(len);
                    let frame = self.frame(direction, at, bytes, Some(kind), Status::Ok);
                    out.push(frame);
                }
                Split::Wait if !flush => break,
                Split::Wait => {
                    if !junk.is_empty() {
                        out.push(unknown(
                            direction,
                            junk_at,
                            std::mem::take(&mut junk),
                            self.lang,
                        ));
                    }
                    let len = self.pending[d].bytes.len();
                    let (at, bytes) = self.pending[d].take(len);
                    out.push(self.leftover(direction, at, bytes));
                }
                Split::Skip => {
                    let (at, byte) = self.pending[d].take(1);
                    if junk.is_empty() {
                        junk_at = at;
                    }
                    junk.extend(byte);
                }
            }
        }
        if !junk.is_empty() {
            out.push(unknown(direction, junk_at, junk, self.lang));
        }
    }

    /// Whether `bytes` begin with a whole message of their own that the
    /// pending bytes would spoil: then what is pending is a message cut
    /// short (or noise) and ends here, before the gap.
    fn starts_afresh(&self, direction: Direction, bytes: &[u8]) -> bool {
        let pending = &self.pending[direction.index()].bytes;
        if pending.is_empty()
            || bytes.is_empty()
            || !matches!(split(bytes, direction), Split::Found(..))
        {
            return false;
        }
        let joined = [pending.as_slice(), bytes].concat();
        !matches!(split(&joined, direction), Split::Found(..))
    }

    /// Bytes that never became a whole message with a good CRC: a message
    /// of the expected length with a bad CRC, an unlisted function code
    /// whose CRC checks out, the start of a message, or nothing known.
    fn leftover(&mut self, direction: Direction, at: u64, bytes: Vec<u8>) -> Frame {
        let fits = candidates(&bytes, direction)
            .into_iter()
            .find(|c| c.len == Some(bytes.len()));
        if let Some(c) = fits {
            return self.frame(direction, at, bytes, Some(c.kind), Status::BadChecksum);
        }
        let known =
            bytes.len() >= 2 && valid_slave(bytes[0]) && name(bytes[1], self.lang).is_some();
        if bytes.len() >= 4 && valid_slave(bytes[0]) && crc_ok(&bytes) {
            return self.frame(direction, at, bytes, None, Status::Ok);
        }
        if known {
            return self.frame(direction, at, bytes, None, Status::Incomplete);
        }
        unknown(direction, at, bytes, self.lang)
    }

    /// A frame of `bytes`, decoded as `kind` (`None`: slave, function and
    /// CRC only).
    fn frame(
        &mut self,
        direction: Direction,
        timestamp_us: u64,
        bytes: Vec<u8>,
        kind: Option<Kind>,
        status: Status,
    ) -> Frame {
        let lang = self.lang;
        let mut f = Fields::new(lang, &bytes);
        let slave = bytes[0];
        let function = bytes.get(1).copied().unwrap_or(0);
        f.byte(
            0,
            t(lang, "slave", "从站"),
            u16::from(slave),
            &slave.to_string(),
        );
        let code = function & 0x7f;
        let fname = name(code, lang).map_or_else(
            || match lang {
                Lang::En => format!("Function 0x{function:02X}"),
                Lang::Zh => format!("功能码 0x{function:02X}"),
            },
            |n| format!("{n} ({function:02X})"),
        );
        if bytes.len() >= 2 {
            f.byte(
                1,
                t(lang, "function", "功能码"),
                u16::from(function),
                &fname,
            );
        }
        let mut frame_name = fname.clone();
        let mut summary = format!("{} {slave}", t(lang, "slave", "从站"));
        let body_end = bytes.len().saturating_sub(2);
        if status == Status::Incomplete {
            summary = match lang {
                Lang::En => format!("{summary} · {} bytes, cut short", bytes.len()),
                Lang::Zh => format!("{summary} · {} 字节，不完整", bytes.len()),
            };
        } else {
            match kind {
                Some(Kind::Exception) => {
                    frame_name = match lang {
                        Lang::En => format!("Exception: {fname}"),
                        Lang::Zh => format!("异常：{fname}"),
                    };
                    let e = bytes[2];
                    let text = format!("{} ({e:02X})", exception(e, lang));
                    f.byte(2, t(lang, "exception", "异常码"), u16::from(e), &text);
                    summary = format!("{summary} · {text}");
                }
                Some(Kind::Request) => {
                    summary.push_str(&self.request_fields(&mut f, slave, function, body_end));
                }
                Some(Kind::Response) => {
                    summary.push_str(&self.response_fields(&mut f, slave, function, body_end));
                }
                None => {
                    if body_end > 2 {
                        f.bytes(2, body_end - 2, t(lang, "data", "数据"));
                    }
                }
            }
            if bytes.len() >= 4 {
                f.crc(body_end);
            }
        }
        if status == Status::BadChecksum {
            summary.push_str(t(lang, " · bad CRC", " · CRC 错误"));
        }
        Frame {
            timestamp_us,
            direction,
            fields: f.list,
            bytes,
            name: frame_name,
            summary,
            status,
        }
    }

    fn request_fields(
        &mut self,
        f: &mut Fields<'_>,
        slave: u8,
        function: u8,
        end: usize,
    ) -> String {
        let lang = self.lang;
        let address = t(lang, "address", "地址");
        let quantity = t(lang, "quantity", "数量");
        match function {
            1..=4 => {
                let a = f.word(2, address);
                let q = f.count(4, quantity);
                self.request = Some(Request {
                    slave,
                    function,
                    address: a,
                    quantity: q,
                });
                format!(" · {address} {} · {quantity} {q}", addr(a))
            }
            5 => {
                let a = f.word(2, address);
                let v = f.coil(4);
                format!(" · {address} {} = {v}", addr(a))
            }
            6 => {
                let a = f.word(2, address);
                let v = f.word(4, t(lang, "value", "值"));
                format!(" · {address} {} = {v}", addr(a))
            }
            8 => {
                let s = f.word(2, t(lang, "sub-function", "子功能"));
                f.word(4, t(lang, "data", "数据"));
                format!(" · {} {s}", t(lang, "sub-function", "子功能"))
            }
            15 | 16 => {
                let a = f.word(2, address);
                let q = f.count(4, quantity);
                f.byte(
                    6,
                    t(lang, "byte count", "字节数"),
                    u16::from(f.at(6)),
                    &f.at(6).to_string(),
                );
                let values = if function == 15 {
                    f.coils(7, end, a, q)
                } else {
                    f.registers(7, end, a)
                };
                format!(" · {address} {} · {values}", addr(a))
            }
            22 => {
                let a = f.word(2, address);
                f.word(4, "AND mask");
                f.word(6, "OR mask");
                format!(" · {address} {}", addr(a))
            }
            23 => {
                let ra = f.word(2, t(lang, "read address", "读地址"));
                let rq = f.count(4, t(lang, "read quantity", "读数量"));
                let wa = f.word(6, t(lang, "write address", "写地址"));
                f.count(8, t(lang, "write quantity", "写数量"));
                f.byte(
                    10,
                    t(lang, "byte count", "字节数"),
                    u16::from(f.at(10)),
                    &f.at(10).to_string(),
                );
                let values = f.registers(11, end, wa);
                self.request = Some(Request {
                    slave,
                    function,
                    address: ra,
                    quantity: rq,
                });
                format!(" · {} {} · {values}", t(lang, "write", "写"), addr(wa))
            }
            _ => String::new(),
        }
    }

    fn response_fields(
        &mut self,
        f: &mut Fields<'_>,
        slave: u8,
        function: u8,
        end: usize,
    ) -> String {
        let lang = self.lang;
        let address = t(lang, "address", "地址");
        let quantity = t(lang, "quantity", "数量");
        let paired = self
            .request
            .filter(|r| r.slave == slave && r.function == function);
        match function {
            1 | 2 | 3 | 4 | 23 | 17 => {
                let n = f.at(2);
                f.byte(
                    2,
                    t(lang, "byte count", "字节数"),
                    u16::from(n),
                    &n.to_string(),
                );
                match function {
                    1 | 2 => {
                        let (start, count) =
                            paired.map_or((0, u16::from(n) * 8), |r| (r.address, r.quantity));
                        format!(" · {}", f.coils(3, end, start, count))
                    }
                    17 => {
                        f.bytes(3, end - 3, t(lang, "data", "数据"));
                        String::new()
                    }
                    _ => {
                        let start = paired.map_or(0, |r| r.address);
                        format!(" · {}", f.registers(3, end, start))
                    }
                }
            }
            5 => {
                let a = f.word(2, address);
                let v = f.coil(4);
                format!(" · {address} {} = {v}", addr(a))
            }
            6 => {
                let a = f.word(2, address);
                let v = f.word(4, t(lang, "value", "值"));
                format!(" · {address} {} = {v}", addr(a))
            }
            7 => {
                let s = f.at(2);
                f.byte(
                    2,
                    t(lang, "status", "状态"),
                    u16::from(s),
                    &format!("0x{s:02X}"),
                );
                format!(" · {} 0x{s:02X}", t(lang, "status", "状态"))
            }
            8 => {
                let s = f.word(2, t(lang, "sub-function", "子功能"));
                f.word(4, t(lang, "data", "数据"));
                format!(" · {} {s}", t(lang, "sub-function", "子功能"))
            }
            11 => {
                f.word(2, t(lang, "status", "状态"));
                let c = f.word(4, t(lang, "event count", "事件数"));
                format!(" · {} {c}", t(lang, "event count", "事件数"))
            }
            15 | 16 => {
                let a = f.word(2, address);
                let q = f.count(4, quantity);
                format!(" · {address} {} · {quantity} {q}", addr(a))
            }
            22 => {
                let a = f.word(2, address);
                f.word(4, "AND mask");
                f.word(6, "OR mask");
                format!(" · {address} {}", addr(a))
            }
            _ => String::new(),
        }
    }
}

impl Decoder for ModbusRtu {
    fn feed(
        &mut self,
        direction: Direction,
        timestamp_us: u64,
        bytes: &[u8],
        out: &mut Vec<Frame>,
    ) {
        let d = direction.index();
        let stale = self.pending[d].stale(timestamp_us, self.gap_us);
        if stale || self.starts_afresh(direction, bytes) {
            self.drain(direction, true, out);
        }
        self.pending[d].push(timestamp_us, bytes);
        self.drain(direction, false, out);
    }

    fn idle(&mut self, now_us: u64, out: &mut Vec<Frame>) {
        for direction in [Direction::Rx, Direction::Tx] {
            if self.pending[direction.index()].stale(now_us, self.gap_us) {
                self.drain(direction, true, out);
            }
        }
    }
}

fn duration_us(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

fn valid_slave(slave: u8) -> bool {
    slave <= 247
}

fn crc_ok(bytes: &[u8]) -> bool {
    let Some(body_end) = bytes.len().checked_sub(2) else {
        return false;
    };
    crc16_modbus(&bytes[..body_end]).to_le_bytes() == bytes[body_end..]
}

/// The messages the buffer may start with, most likely first.
fn candidates(buf: &[u8], direction: Direction) -> Vec<Candidate> {
    let byte = |i: usize| buf.get(i).map(|b| usize::from(*b));
    let fixed = |kind, len| Candidate {
        kind,
        len: Some(len),
    };
    let counted = |kind, at: usize, plus: usize| Candidate {
        kind,
        len: byte(at).map(|n| n + plus),
    };
    let Some(function) = buf.get(1).copied() else {
        return Vec::new();
    };
    if function & 0x80 != 0 {
        return if name(function & 0x7f, Lang::En).is_some() {
            vec![fixed(Kind::Exception, 5)]
        } else {
            Vec::new()
        };
    }
    let (request, response) = match function {
        1..=4 => (fixed(Kind::Request, 8), counted(Kind::Response, 2, 5)),
        5 | 6 | 8 => (fixed(Kind::Request, 8), fixed(Kind::Response, 8)),
        7 => (fixed(Kind::Request, 4), fixed(Kind::Response, 5)),
        11 => (fixed(Kind::Request, 4), fixed(Kind::Response, 8)),
        15 | 16 => (counted(Kind::Request, 6, 9), fixed(Kind::Response, 8)),
        17 => (fixed(Kind::Request, 4), counted(Kind::Response, 2, 5)),
        22 => (fixed(Kind::Request, 10), fixed(Kind::Response, 10)),
        23 => (
            counted(Kind::Request, 10, 13),
            counted(Kind::Response, 2, 5),
        ),
        _ => return Vec::new(),
    };
    match direction {
        Direction::Tx => vec![request, response],
        Direction::Rx => vec![response, request],
    }
}

fn split(buf: &[u8], direction: Direction) -> Split {
    if !valid_slave(buf[0]) {
        return Split::Skip;
    }
    if buf.len() < 2 {
        return Split::Wait;
    }
    let candidates = candidates(buf, direction);
    if candidates.is_empty() {
        // An unlisted function code: wait for the gap, within reason.
        return if name(buf[1] & 0x7f, Lang::En).is_none() && buf[1] != 0 && buf.len() < MAX_LEN {
            Split::Wait
        } else {
            Split::Skip
        };
    }
    for c in &candidates {
        if let Some(len) = c.len
            && len <= buf.len()
            && crc_ok(&buf[..len])
        {
            return Split::Found(len, c.kind);
        }
    }
    // More bytes may come, or a whole message with a bad CRC may be all
    // there is: the gap decides.
    let more = candidates
        .iter()
        .any(|c| c.len.is_none_or(|len| len >= buf.len() && len <= MAX_LEN));
    if more { Split::Wait } else { Split::Skip }
}

fn t(lang: Lang, en: &'static str, zh: &'static str) -> &'static str {
    match lang {
        Lang::En => en,
        Lang::Zh => zh,
    }
}

/// The name of a public function code.
fn name(function: u8, lang: Lang) -> Option<&'static str> {
    Some(match function {
        1 => t(lang, "Read Coils", "读线圈"),
        2 => t(lang, "Read Discrete Inputs", "读离散输入"),
        3 => t(lang, "Read Holding Registers", "读保持寄存器"),
        4 => t(lang, "Read Input Registers", "读输入寄存器"),
        5 => t(lang, "Write Single Coil", "写单个线圈"),
        6 => t(lang, "Write Single Register", "写单个寄存器"),
        7 => t(lang, "Read Exception Status", "读异常状态"),
        8 => t(lang, "Diagnostics", "诊断"),
        11 => t(lang, "Get Comm Event Counter", "读通信事件计数"),
        15 => t(lang, "Write Multiple Coils", "写多个线圈"),
        16 => t(lang, "Write Multiple Registers", "写多个寄存器"),
        17 => t(lang, "Report Server ID", "报告从站 ID"),
        22 => t(lang, "Mask Write Register", "掩码写寄存器"),
        23 => t(lang, "Read/Write Multiple Registers", "读写多个寄存器"),
        _ => return None,
    })
}

fn exception(code: u8, lang: Lang) -> &'static str {
    match code {
        1 => t(lang, "illegal function", "非法功能"),
        2 => t(lang, "illegal data address", "非法数据地址"),
        3 => t(lang, "illegal data value", "非法数据值"),
        4 => t(lang, "server device failure", "从站设备故障"),
        5 => t(lang, "acknowledge", "已确认，处理中"),
        6 => t(lang, "server device busy", "从站设备忙"),
        8 => t(lang, "memory parity error", "存储奇偶校验错误"),
        10 => t(lang, "gateway path unavailable", "网关路径不可用"),
        11 => t(lang, "gateway target failed to respond", "网关目标无响应"),
        _ => t(lang, "unknown exception", "未知异常"),
    }
}

/// A register or coil address: decimal and hex.
fn addr(a: u16) -> String {
    format!("{a} (0x{a:04X})")
}

/// Builds a frame's fields.
struct Fields<'a> {
    lang: Lang,
    bytes: &'a [u8],
    list: Vec<Field>,
}

impl<'a> Fields<'a> {
    fn new(lang: Lang, bytes: &'a [u8]) -> Self {
        Self {
            lang,
            bytes,
            list: Vec::new(),
        }
    }

    fn at(&self, i: usize) -> u8 {
        self.bytes.get(i).copied().unwrap_or(0)
    }

    fn be16(&self, i: usize) -> u16 {
        u16::from_be_bytes([self.at(i), self.at(i + 1)])
    }

    fn push(&mut self, name: &str, offset: usize, len: usize, value: Value, text: String) {
        self.list.push(Field {
            name: name.to_owned(),
            offset,
            len,
            value,
            text,
        });
    }

    fn byte(&mut self, i: usize, name: &str, value: u16, text: &str) {
        self.push(name, i, 1, Value::Int(i64::from(value)), text.to_owned());
    }

    /// A big-endian 16-bit address or value, shown in decimal and hex.
    fn word(&mut self, i: usize, name: &str) -> u16 {
        let v = self.be16(i);
        self.push(name, i, 2, Value::Int(i64::from(v)), addr(v));
        v
    }

    /// A big-endian 16-bit count, shown in decimal.
    fn count(&mut self, i: usize, name: &str) -> u16 {
        let v = self.be16(i);
        self.push(name, i, 2, Value::Int(i64::from(v)), v.to_string());
        v
    }

    /// A single coil's value: `FF00` on, `0000` off.
    fn coil(&mut self, i: usize) -> String {
        let v = self.be16(i);
        let text = match v {
            0xFF00 => "ON".to_owned(),
            0x0000 => "OFF".to_owned(),
            _ => format!("0x{v:04X} ({})", t(self.lang, "invalid", "无效")),
        };
        let name = t(self.lang, "value", "值");
        self.push(name, i, 2, Value::Bool(v == 0xFF00), text.clone());
        text
    }

    fn bytes(&mut self, i: usize, len: usize, name: &str) {
        let end = (i + len).min(self.bytes.len());
        let slice = &self.bytes[i.min(end)..end];
        let text = crate::hex(slice);
        self.push(name, i, slice.len(), Value::Text(text.clone()), text);
    }

    /// Registers from byte `from` to `end`, one field each, numbered from
    /// `start`. Returns the summary's `address=value` list.
    fn registers(&mut self, from: usize, end: usize, start: u16) -> String {
        let mut summary = Vec::new();
        let mut i = from;
        let mut n = 0u16;
        while i + 1 < end {
            let a = start.wrapping_add(n);
            let v = self.be16(i);
            let name = format!("{} {a}", t(self.lang, "register", "寄存器"));
            self.push(
                &name,
                i,
                2,
                Value::Int(i64::from(v)),
                format!("{v} (0x{v:04X})"),
            );
            if summary.len() < SUMMARY_VALUES {
                summary.push(format!("{a}={v}"));
            } else if summary.len() == SUMMARY_VALUES {
                summary.push("…".to_owned());
            }
            i += 2;
            n = n.wrapping_add(1);
        }
        summary.join(" ")
    }

    /// `count` coils packed in the bytes from `from` to `end` (first coil in
    /// the lowest bit), as one field. Returns the summary's bits.
    fn coils(&mut self, from: usize, end: usize, start: u16, count: u16) -> String {
        let available = end.saturating_sub(from) * 8;
        let count = usize::from(count).min(available);
        let bits: Vec<i64> = (0..count)
            .map(|k| i64::from((self.at(from + k / 8) >> (k % 8)) & 1))
            .collect();
        let text = bits
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        let name = format!("{} {start}…", t(self.lang, "coils", "线圈"));
        self.push(
            &name,
            from,
            end.saturating_sub(from),
            Value::List(bits),
            text.clone(),
        );
        let mut summary: String = text.chars().take(SUMMARY_VALUES * 2).collect();
        if text.chars().count() > SUMMARY_VALUES * 2 {
            summary = format!("{}…", summary.trim_end());
        }
        format!("{} {start}: {summary}", t(self.lang, "from", "起始"))
    }

    /// The CRC at `at`: shown as sent, and what it should be when wrong.
    fn crc(&mut self, at: usize) {
        let sent = u16::from_le_bytes([self.at(at), self.at(at + 1)]);
        let want = crc16_modbus(&self.bytes[..at]);
        let text = if sent == want {
            format!("0x{sent:04X} ✓")
        } else {
            match self.lang {
                Lang::En => format!("0x{sent:04X}, should be 0x{want:04X}"),
                Lang::Zh => format!("0x{sent:04X}，应为 0x{want:04X}"),
            }
        };
        self.push("CRC", at, 2, Value::Int(i64::from(sent)), text);
    }
}

#[cfg(test)]
mod tests;
