// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! SLCAN, the Lawicel ASCII protocol of serial CAN adapters (`CANable`,
//! `CANtact`, many USB-CAN sticks): one line per message, ended by `\r`; an
//! adapter answers a failed command with BEL (`\a`).
//!
//! CAN frames are `t` (standard, 3 hex digits of ID), `T` (extended, 8),
//! `r` / `R` (remote), then the length and the data in hex, then an
//! optional 4-digit timestamp in milliseconds. Configuration commands
//! (`S6`, `O`, `C`, `V`, …) and the adapter's replies are named too. A line
//! that does not fit the protocol comes out as unknown, whole.

use std::time::Duration;

use crate::{Decoder, Direction, Field, Frame, Lang, Pending, Status, Value, unknown};

/// The longest line: `T` + 8 + 1 + 16 + 4 digits, with room to spare.
const MAX_LINE: usize = 64;

/// Bit rates of `S0`–`S8`, in kbit/s.
const BIT_RATES: [u32; 9] = [10, 20, 50, 100, 125, 250, 500, 800, 1000];

/// An SLCAN decoder.
#[derive(Debug)]
pub struct Slcan {
    lang: Lang,
    gap_us: u64,
    pending: [Pending; 2],
}

impl Slcan {
    /// A line left unfinished this long is given back as incomplete.
    pub const DEFAULT_GAP: Duration = Duration::from_millis(200);

    /// A decoder writing names and summaries in `lang`.
    pub fn new(lang: Lang) -> Self {
        Self {
            lang,
            gap_us: u64::try_from(Self::DEFAULT_GAP.as_micros()).unwrap_or(u64::MAX),
            pending: [Pending::default(), Pending::default()],
        }
    }

    fn drain(&mut self, direction: Direction, flush: bool, out: &mut Vec<Frame>) {
        let d = direction.index();
        loop {
            let bytes = &self.pending[d].bytes;
            if bytes.is_empty() {
                break;
            }
            if let Some(end) = bytes.iter().position(|b| *b == b'\r' || *b == 0x07) {
                let (at, line) = self.pending[d].take(end + 1);
                out.push(self.line(direction, at, line));
            } else if bytes.len() > MAX_LINE {
                let (at, line) = self.pending[d].take(bytes.len());
                out.push(unknown(direction, at, line, self.lang));
            } else if flush {
                let (at, line) = self.pending[d].take(bytes.len());
                let mut frame = unknown(direction, at, line, self.lang);
                frame.status = Status::Incomplete;
                self.t("Unfinished line", "未结束的行")
                    .clone_into(&mut frame.name);
                out.push(frame);
            } else {
                break;
            }
        }
    }

    fn t(&self, en: &'static str, zh: &'static str) -> &'static str {
        match self.lang {
            Lang::En => en,
            Lang::Zh => zh,
        }
    }

    /// One line with its terminator.
    fn line(&self, direction: Direction, timestamp_us: u64, bytes: Vec<u8>) -> Frame {
        let decoded = if bytes.last() == Some(&0x07) {
            Some(Self::reply(
                self.t("Error", "错误"),
                self.t("command failed", "命令失败"),
            ))
        } else {
            let text = &bytes[..bytes.len() - 1];
            std::str::from_utf8(text)
                .ok()
                .and_then(|text| self.decode(direction, text))
        };
        match decoded {
            Some((name, summary, fields)) => Frame {
                timestamp_us,
                direction,
                bytes,
                name,
                summary,
                fields,
                status: Status::Ok,
            },
            None => unknown(direction, timestamp_us, bytes, self.lang),
        }
    }

    fn reply(name: &str, summary: &str) -> (String, String, Vec<Field>) {
        (name.to_owned(), summary.to_owned(), Vec::new())
    }

    fn decode(&self, direction: Direction, text: &str) -> Option<(String, String, Vec<Field>)> {
        let mut chars = text.chars();
        let Some(first) = chars.next() else {
            return Some(Self::reply("OK", self.t("command done", "命令完成")));
        };
        let rest = chars.as_str();
        match first {
            't' | 'T' | 'r' | 'R' => self.can(first, rest),
            'z' | 'Z' if rest.is_empty() && direction == Direction::Rx => Some(Self::reply(
                self.t("Transmitted", "已发送"),
                self.t("frame queued for the bus", "帧已进入发送队列"),
            )),
            'S' => {
                let i = rest.parse::<usize>().ok().filter(|_| rest.len() == 1)?;
                let rate = BIT_RATES.get(i)?;
                let summary = if *rate >= 1000 {
                    "1 Mbit/s".to_owned()
                } else {
                    format!("{rate} kbit/s")
                };
                let field = field("S", 1, 1, Value::Int(i64::from(*rate) * 1000), &summary);
                Some((
                    self.t("Set bit rate", "设置波特率").to_owned(),
                    summary,
                    vec![field],
                ))
            }
            's' if hex_ok(rest, 4) => {
                Some(self.command("Set bit timing (BTR)", "设置位时序（BTR）", rest))
            }
            'O' if rest.is_empty() => Some(self.command("Open channel", "打开通道", "")),
            'L' if rest.is_empty() => Some(self.command("Open listen-only", "只听模式打开", "")),
            'C' if rest.is_empty() => Some(self.command("Close channel", "关闭通道", "")),
            'V' | 'v' if rest.is_empty() => Some(self.command("Get version", "查询版本", "")),
            'V' | 'v' if hex_ok(rest, rest.len()) => Some(self.command("Version", "版本", rest)),
            'N' if rest.is_empty() => Some(self.command("Get serial number", "查询序列号", "")),
            'N' => Some(self.command("Serial number", "序列号", rest)),
            'F' if rest.is_empty() => Some(self.command("Read status flags", "读状态标志", "")),
            'F' if hex_ok(rest, 2) => {
                Some(self.command("Status flags", "状态标志", &format!("0x{rest}")))
            }
            'Z' if rest == "0" || rest == "1" => {
                let on = self.t(
                    if rest == "1" { "on" } else { "off" },
                    if rest == "1" { "开" } else { "关" },
                );
                Some(self.command("Timestamps", "时间戳", on))
            }
            'M' if hex_ok(rest, 8) => {
                Some(self.command("Acceptance code", "验收码", &format!("0x{rest}")))
            }
            'm' if hex_ok(rest, 8) => {
                Some(self.command("Acceptance mask", "验收屏蔽码", &format!("0x{rest}")))
            }
            _ => None,
        }
    }

    fn command(
        &self,
        en: &'static str,
        zh: &'static str,
        summary: &str,
    ) -> (String, String, Vec<Field>) {
        (self.t(en, zh).to_owned(), summary.to_owned(), Vec::new())
    }

    /// A CAN frame: `t`/`r` with a 3-digit ID, `T`/`R` with 8 digits.
    fn can(&self, kind: char, rest: &str) -> Option<(String, String, Vec<Field>)> {
        let extended = kind.is_ascii_uppercase();
        let remote = kind == 'r' || kind == 'R';
        let id_len = if extended { 8 } else { 3 };
        let id_text = rest.get(..id_len)?;
        let id = u32::from_str_radix(id_text, 16)
            .ok()
            .filter(|_| hex_ok(id_text, id_len))?;
        if id > if extended { 0x1FFF_FFFF } else { 0x7FF } {
            return None;
        }
        let dlc_text = rest.get(id_len..=id_len)?;
        let dlc = usize::from_str_radix(dlc_text, 16)
            .ok()
            .filter(|n| *n <= 8)?;
        let data_len = if remote { 0 } else { dlc * 2 };
        let data_text = rest.get(id_len + 1..id_len + 1 + data_len)?;
        if !hex_ok(data_text, data_len) {
            return None;
        }
        let tail = &rest[id_len + 1 + data_len..];
        let stamp = match tail.len() {
            0 => None,
            4 if hex_ok(tail, 4) => u16::from_str_radix(tail, 16).ok(),
            _ => return None,
        };
        let data: Vec<u8> = (0..data_len / 2)
            .filter_map(|i| u8::from_str_radix(&data_text[i * 2..i * 2 + 2], 16).ok())
            .collect();
        let name = match (extended, remote) {
            (false, false) => self.t("CAN standard frame", "CAN 标准帧"),
            (true, false) => self.t("CAN extended frame", "CAN 扩展帧"),
            (false, true) => self.t("CAN remote frame (standard)", "CAN 远程帧（标准）"),
            (true, true) => self.t("CAN remote frame (extended)", "CAN 远程帧（扩展）"),
        };
        let id_shown = if extended {
            format!("0x{id:08X}")
        } else {
            format!("0x{id:03X}")
        };
        let data_shown = crate::hex(&data);
        let mut fields = vec![
            field("ID", 1, id_len, Value::Int(i64::from(id)), &id_shown),
            field(
                "DLC",
                1 + id_len,
                1,
                Value::Int(i64::try_from(dlc).ok()?),
                &dlc.to_string(),
            ),
        ];
        let mut summary = format!("ID {id_shown} · DLC {dlc}");
        if !remote {
            let name = self.t("data", "数据");
            fields.push(field(
                name,
                2 + id_len,
                data_len,
                Value::Text(data_shown.clone()),
                &data_shown,
            ));
            if !data.is_empty() {
                summary = format!("{summary} · {data_shown}");
            }
        }
        if let Some(ms) = stamp {
            let name = self.t("timestamp", "时间戳");
            let at = 2 + id_len + data_len;
            fields.push(field(
                name,
                at,
                4,
                Value::Int(i64::from(ms)),
                &format!("{ms} ms"),
            ));
            summary = format!("{summary} · {ms} ms");
        }
        Some((name.to_owned(), summary, fields))
    }
}

fn field(name: &str, offset: usize, len: usize, value: Value, text: &str) -> Field {
    Field {
        name: name.to_owned(),
        offset,
        len,
        value,
        text: text.to_owned(),
    }
}

/// Whether `text` is exactly `len` hex digits.
fn hex_ok(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Decoder for Slcan {
    fn feed(
        &mut self,
        direction: Direction,
        timestamp_us: u64,
        bytes: &[u8],
        out: &mut Vec<Frame>,
    ) {
        let d = direction.index();
        if self.pending[d].stale(timestamp_us, self.gap_us) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode_all;

    fn decode(lang: Lang, chunks: &[(Direction, &[u8])]) -> Vec<Frame> {
        let mut d = Slcan::new(lang);
        decode_all(&mut d, chunks.iter().map(|(dir, b)| (*dir, 0, *b)))
    }

    fn one(dir: Direction, line: &str) -> Frame {
        let frames = decode(Lang::En, &[(dir, line.as_bytes())]);
        assert_eq!(frames.len(), 1, "{line:?}: {frames:#?}");
        frames.into_iter().next().unwrap()
    }

    #[test]
    fn can_frames_show_id_length_data_and_time() {
        let f = one(Direction::Rx, "t1233112233\r");
        assert_eq!(
            (f.name.as_str(), f.status),
            ("CAN standard frame", Status::Ok)
        );
        assert_eq!(f.summary, "ID 0x123 · DLC 3 · 11 22 33");
        assert_eq!(f.fields[0].value, Value::Int(0x123));
        assert_eq!((f.fields[2].offset, f.fields[2].len), (5, 6));

        let f = one(Direction::Rx, "T1FFFFFFF8DEADBEEF0011223304D2\r");
        assert_eq!(f.name, "CAN extended frame");
        assert_eq!(
            f.summary,
            "ID 0x1FFFFFFF · DLC 8 · DE AD BE EF 00 11 22 33 · 1234 ms"
        );
        assert_eq!(f.fields[3].name, "timestamp");

        assert_eq!(one(Direction::Tx, "r7FF0\r").summary, "ID 0x7FF · DLC 0");
        assert_eq!(
            one(Direction::Tx, "R000000018\r").name,
            "CAN remote frame (extended)"
        );
        assert_eq!(one(Direction::Rx, "t0000\r").summary, "ID 0x000 · DLC 0");
    }

    #[test]
    fn commands_and_replies_are_named() {
        for (dir, line, name, summary) in [
            (Direction::Tx, "S6\r", "Set bit rate", "500 kbit/s"),
            (Direction::Tx, "S8\r", "Set bit rate", "1 Mbit/s"),
            (Direction::Tx, "s031C\r", "Set bit timing (BTR)", "031C"),
            (Direction::Tx, "O\r", "Open channel", ""),
            (Direction::Tx, "L\r", "Open listen-only", ""),
            (Direction::Tx, "C\r", "Close channel", ""),
            (Direction::Tx, "V\r", "Get version", ""),
            (Direction::Rx, "V1013\r", "Version", "1013"),
            (Direction::Tx, "N\r", "Get serial number", ""),
            (Direction::Rx, "NA123\r", "Serial number", "A123"),
            (Direction::Tx, "F\r", "Read status flags", ""),
            (Direction::Rx, "F00\r", "Status flags", "0x00"),
            (Direction::Tx, "Z1\r", "Timestamps", "on"),
            (
                Direction::Tx,
                "M00000000\r",
                "Acceptance code",
                "0x00000000",
            ),
            (
                Direction::Tx,
                "mFFFFFFFF\r",
                "Acceptance mask",
                "0xFFFFFFFF",
            ),
            (Direction::Rx, "\r", "OK", "command done"),
            (
                Direction::Rx,
                "z\r",
                "Transmitted",
                "frame queued for the bus",
            ),
            (Direction::Rx, "\x07", "Error", "command failed"),
        ] {
            let f = one(dir, line);
            assert_eq!(
                (f.name.as_str(), f.summary.as_str(), f.status),
                (name, summary, Status::Ok),
                "{line:?}"
            );
        }
    }

    #[test]
    fn lines_that_do_not_fit_are_unknown() {
        for line in [
            "t12\r",
            "t1239\r",
            "t800\r",
            "T200000000\r",
            "t1232AAB\r",
            "t1231G0\r",
            "t1230123\r",
            "S9\r",
            "S\r",
            "X\r",
            "z\r",
            "\u{e9}\r",
        ] {
            let dir = if line == "z\r" {
                Direction::Tx
            } else {
                Direction::Rx
            };
            assert_eq!(one(dir, line).status, Status::Unknown, "{line:?}");
        }
    }

    #[test]
    fn lines_split_anywhere_and_unfinished_ones_come_back() {
        let frames = decode(
            Lang::En,
            &[
                (Direction::Rx, b"t12"),
                (Direction::Rx, b"31AA\rt1"),
                (Direction::Rx, b"001BB\rt45"),
            ],
        );
        assert_eq!(frames.len(), 3, "{frames:#?}");
        assert_eq!(frames[0].summary, "ID 0x123 · DLC 1 · AA");
        assert_eq!(frames[1].summary, "ID 0x100 · DLC 1 · BB");
        assert_eq!(frames[2].status, Status::Incomplete);
        assert_eq!(frames[2].name, "Unfinished line");
        let long = vec![b'7'; MAX_LINE + 1];
        let frames = decode(Lang::En, &[(Direction::Rx, &long)]);
        assert_eq!(frames[0].status, Status::Unknown);

        // A line still waiting is given back after the gap.
        let mut d = Slcan::new(Lang::En);
        let mut out = Vec::new();
        d.feed(Direction::Tx, 0, b"t1", &mut out);
        d.idle(100_000, &mut out);
        assert!(out.is_empty());
        d.feed(Direction::Tx, 500_000, b"t1231AA\r", &mut out);
        assert_eq!(out.len(), 2, "{out:#?}");
        assert_eq!(out[0].bytes, b"t1");
    }

    #[test]
    fn chinese_names() {
        let frames = decode(
            Lang::Zh,
            &[(
                Direction::Rx,
                b"t1231AA\rT000000010\rr1230\rR000000010\rS6\r\x07t1",
            )],
        );
        let names: Vec<&str> = frames.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "CAN 标准帧",
                "CAN 扩展帧",
                "CAN 远程帧（标准）",
                "CAN 远程帧（扩展）",
                "设置波特率",
                "错误",
                "未结束的行"
            ]
        );
        assert_eq!(frames[0].fields[2].name, "数据");
    }
}
