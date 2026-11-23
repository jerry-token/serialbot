// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Log entries to the lines the interactive terminal prints.
//!
//! Received bytes are decoded in the chosen encoding (a character split
//! across reads stays whole), cut into lines, and given the chosen
//! timestamps. Only color sequences (`ESC [ … m`) from the device are
//! kept: cursor moves and screen clears would break the input box drawn
//! below the output, so they are dropped.

use std::fmt::Write as _;

use encoding_rs::{Decoder, Encoding};
use serialbot_core::hex;
use serialbot_core::log::{Actor, Direction, Entry, EntryKind};
use serialbot_jobs::export::format_timestamp;

/// Timestamps at the start of each line (minicom's `Ctrl+A N`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stamps {
    /// None.
    Off,
    /// Local time of the line's first byte, `[HH:MM:SS.mmm]`.
    Time,
    /// Time since the previous line started, `[+0.123]`.
    Delta,
}

impl Stamps {
    /// The next mode, as `Ctrl+A N` cycles: off → time → delta → off.
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Time,
            Self::Time => Self::Delta,
            Self::Delta => Self::Off,
        }
    }
}

/// How received data is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Display {
    /// Bytes as hex, one line per read.
    pub hex: bool,
    /// Timestamps.
    pub stamps: Stamps,
    /// Show sent data (`→ …` lines).
    pub echo: bool,
    /// A lone carriage return also ends the line (minicom `Ctrl+A A`).
    pub add_lf: bool,
}

impl Default for Display {
    fn default() -> Self {
        Self {
            hex: false,
            stamps: Stamps::Off,
            echo: true,
            add_lf: false,
        }
    }
}

const DIM: &str = "\x1b[2m";
const CYAN: &str = "\x1b[36m";
const MAGENTA: &str = "\x1b[35m";
const RESET: &str = "\x1b[0m";

/// Turns log entries into printable lines.
pub struct Feed {
    /// How data is shown; may change at any time.
    pub display: Display,
    /// Whether to emit color sequences.
    pub color: bool,
    utc_offset_min: i32,
    decoder: Decoder,
    /// The received line not finished yet.
    pending: String,
    /// When the pending line started (µs since the epoch).
    pending_since: Option<u64>,
    /// When the previous line started, for [`Stamps::Delta`].
    last_start: Option<u64>,
    /// The last received character was a carriage return.
    after_cr: bool,
    /// An escape sequence cut by a read: the part seen so far.
    escape: Option<String>,
}

impl std::fmt::Debug for Feed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Feed")
            .field("display", &self.display)
            .field("pending", &self.pending)
            .finish_non_exhaustive()
    }
}

impl Feed {
    /// A feed decoding received text as `encoding` (a WHATWG label such as
    /// `utf-8` or `gbk`); `None` for an unknown label.
    pub fn new(encoding: &str, utc_offset_min: i32, color: bool) -> Option<Self> {
        let encoding = Encoding::for_label(encoding.trim().as_bytes())?;
        Some(Self {
            display: Display::default(),
            color,
            utc_offset_min,
            decoder: encoding.new_decoder_without_bom_handling(),
            pending: String::new(),
            pending_since: None,
            last_start: None,
            after_cr: false,
            escape: None,
        })
    }

    /// The finished lines one entry adds (without line breaks).
    pub fn entry(&mut self, entry: &Entry) -> Vec<String> {
        match &entry.kind {
            EntryKind::Data {
                direction: Direction::Rx,
                bytes,
            } => {
                if self.display.hex {
                    let line =
                        self.paint(DIM, &self.stamp(entry.timestamp_us)) + &hex::to_hex(bytes);
                    self.last_start = Some(entry.timestamp_us);
                    return vec![line];
                }
                self.received(bytes, entry.timestamp_us)
            }
            EntryKind::Data {
                direction: Direction::Tx,
                bytes,
            } => {
                if !self.display.echo {
                    return Vec::new();
                }
                let mut lines = self.break_pending();
                let body = if self.display.hex {
                    hex::to_hex(bytes)
                } else {
                    escape_controls(&String::from_utf8_lossy(bytes))
                };
                let badge = if entry.actor == Actor::Ai {
                    self.paint(MAGENTA, "AI ")
                } else {
                    String::new()
                };
                let stamp = self.paint(DIM, &self.stamp(entry.timestamp_us));
                lines.push(format!(
                    "{stamp}{badge}{}",
                    self.paint(CYAN, &format!("→ {body}"))
                ));
                lines
            }
            EntryKind::Marker { label } => {
                let mut lines = self.break_pending();
                lines.push(self.paint(DIM, &format!("# {label}")));
                lines
            }
        }
    }

    /// A note from the program itself (not the device), as a line.
    pub fn note(&mut self, text: &str) -> Vec<String> {
        let mut lines = self.break_pending();
        lines.push(self.paint(DIM, &format!("· {text}")));
        lines
    }

    /// The unfinished received line, with its timestamp, if any.
    pub fn pending(&self) -> Option<String> {
        let since = self.pending_since?;
        let stamp = self.paint(DIM, &self.stamp_at(since, self.last_start));
        Some(format!(
            "{stamp}{}{}",
            self.pending,
            if self.color { RESET } else { "" }
        ))
    }

    /// Ends the unfinished line (before something else is printed, so it
    /// is not mixed with it).
    fn break_pending(&mut self) -> Vec<String> {
        let Some(line) = self.pending() else {
            return Vec::new();
        };
        self.pending.clear();
        self.last_start = self.pending_since.take();
        vec![line]
    }

    fn received(&mut self, bytes: &[u8], at: u64) -> Vec<String> {
        let mut text = String::with_capacity(
            self.decoder
                .max_utf8_buffer_length(bytes.len())
                .unwrap_or(bytes.len()),
        );
        let _ = self.decoder.decode_to_string(bytes, &mut text, false);
        let mut lines = Vec::new();
        for c in text.chars() {
            if let Some(seq) = self.escape.as_mut() {
                seq.push(c);
                if seq.len() == 2 && c != '[' {
                    // A two-character escape (ESC c, ESC 7, …): dropped.
                    self.escape = None;
                } else if seq.len() > 2 && ('\u{40}'..='\u{7e}').contains(&c) {
                    let done = self.escape.take().unwrap_or_default();
                    if c == 'm' && self.color {
                        self.push(&done, at);
                    }
                }
                continue;
            }
            match c {
                '\u{1b}' => self.escape = Some(c.to_string()),
                '\n' => {
                    self.after_cr = false;
                    lines.push(self.finish_line(at));
                }
                '\r' => self.after_cr = true,
                c => {
                    if self.after_cr {
                        self.after_cr = false;
                        if self.display.add_lf {
                            lines.push(self.finish_line(at));
                        } else {
                            // A carriage return alone rewrites the line
                            // (progress bars): keep what follows.
                            self.pending.clear();
                        }
                    }
                    if c == '\t' || !c.is_control() {
                        self.push(&c.to_string(), at);
                    }
                }
            }
        }
        if self.after_cr && self.display.add_lf {
            self.after_cr = false;
            lines.push(self.finish_line(at));
        }
        lines
    }

    fn push(&mut self, text: &str, at: u64) {
        if self.pending_since.is_none() {
            self.pending_since = Some(at);
        }
        self.pending.push_str(text);
    }

    fn finish_line(&mut self, at: u64) -> String {
        let since = self.pending_since.take().unwrap_or(at);
        let stamp = self.paint(DIM, &self.stamp_at(since, self.last_start));
        self.last_start = Some(since);
        let reset = if self.color { RESET } else { "" };
        format!("{stamp}{}{reset}", std::mem::take(&mut self.pending))
    }

    fn stamp(&self, at: u64) -> String {
        self.stamp_at(at, self.last_start)
    }

    fn stamp_at(&self, at: u64, previous: Option<u64>) -> String {
        match self.display.stamps {
            Stamps::Off => String::new(),
            Stamps::Time => {
                let full = format_timestamp(at, self.utc_offset_min);
                format!("[{}] ", full.get(11..23).unwrap_or(&full))
            }
            Stamps::Delta => {
                let gap = previous.map_or(0, |p| at.saturating_sub(p));
                format!("[+{}.{:03}] ", gap / 1_000_000, gap % 1_000_000 / 1000)
            }
        }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.color && !text.is_empty() {
            format!("{code}{text}{RESET}")
        } else {
            text.to_owned()
        }
    }
}

/// Control characters of a sent line made visible (`\r`, `\n`, `\x01`).
fn escape_controls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\r' => out.push_str("\\r"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\x{:02x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    /// 2026-11-18 10:13:38.123456 UTC.
    const T0: u64 = 1_794_996_818_123_456;

    fn rx(at: u64, data: &[u8]) -> Entry {
        Entry {
            seq: 0,
            timestamp_us: at,
            kind: EntryKind::Data {
                direction: Direction::Rx,
                bytes: Bytes::copy_from_slice(data),
            },
            actor: Actor::User,
        }
    }

    fn tx(actor: Actor, data: &[u8]) -> Entry {
        Entry {
            kind: EntryKind::Data {
                direction: Direction::Tx,
                bytes: Bytes::copy_from_slice(data),
            },
            actor,
            ..rx(T0, b"")
        }
    }

    fn feed() -> Feed {
        Feed::new("utf-8", 0, false).unwrap()
    }

    #[test]
    fn lines_are_cut_at_line_feeds_across_reads() {
        let mut f = feed();
        assert!(f.entry(&rx(T0, b"boot ")).is_empty());
        assert_eq!(f.pending().as_deref(), Some("boot "));
        assert_eq!(f.entry(&rx(T0, b"ok\r\nuart:~$ ")), ["boot ok"]);
        assert_eq!(f.pending().as_deref(), Some("uart:~$ "));
        assert_eq!(f.entry(&rx(T0, b"\n\n")), ["uart:~$ ", ""]);
        assert_eq!(f.pending(), None);
        assert!(format!("{f:?}").contains("Feed"));
    }

    #[test]
    fn characters_split_across_reads_stay_whole() {
        let mut f = feed();
        let text = "温度\n".as_bytes();
        assert!(f.entry(&rx(T0, &text[..2])).is_empty());
        assert_eq!(f.entry(&rx(T0, &text[2..])), ["温度"]);
        let mut gbk = Feed::new("gbk", 0, false).unwrap();
        assert_eq!(gbk.entry(&rx(T0, &[0xd6, 0xd0, b'\n'])), ["中"]);
        assert!(Feed::new("nope", 0, false).is_none());
    }

    #[test]
    fn carriage_returns_rewrite_or_end_the_line() {
        let mut f = feed();
        f.entry(&rx(T0, b"10%\r20%\r"));
        assert_eq!(f.pending().as_deref(), Some("20%"), "progress overwrites");
        assert_eq!(f.entry(&rx(T0, b"\n")), ["20%"], "CR LF split across reads");
        f.display.add_lf = true;
        assert_eq!(f.entry(&rx(T0, b"a\rb\r")), ["a", "b"]);
    }

    #[test]
    fn only_color_sequences_are_kept() {
        let mut plain = feed();
        assert_eq!(
            plain.entry(&rx(T0, b"\x1b[1;32muart\x1b[m \x1b[2J\x1b[H\x1bcok\n")),
            ["uart ok"]
        );
        let mut colored = Feed::new("utf-8", 0, true).unwrap();
        // A sequence split across reads is still recognized.
        colored.entry(&rx(T0, b"\x1b[1;3"));
        assert_eq!(
            colored.entry(&rx(T0, b"2mOK\x1b[K\x07\n")),
            ["\x1b[1;32mOK\x1b[0m"],
            "color kept, erase line and bell dropped"
        );
    }

    #[test]
    fn timestamps_cycle_through_time_and_delta() {
        assert_eq!(Stamps::Off.next(), Stamps::Time);
        assert_eq!(Stamps::Time.next(), Stamps::Delta);
        assert_eq!(Stamps::Delta.next(), Stamps::Off);
        let mut f = Feed::new("utf-8", 480, false).unwrap();
        f.display.stamps = Stamps::Time;
        assert_eq!(f.entry(&rx(T0, b"a\n")), ["[18:13:38.123] a"]);
        f.display.stamps = Stamps::Delta;
        f.entry(&rx(T0 + 250_000, b"b"));
        assert_eq!(f.pending().as_deref(), Some("[+0.250] b"));
        assert_eq!(
            f.entry(&rx(T0 + 300_000, b"\n")),
            ["[+0.250] b"],
            "the line's first byte counts"
        );
        assert_eq!(f.entry(&rx(T0 + 1_300_000, b"c\n")), ["[+1.050] c"]);
    }

    #[test]
    fn hex_shows_each_read() {
        let mut f = feed();
        f.display.hex = true;
        assert_eq!(f.entry(&rx(T0, &[0x01, 0xff])), ["01 ff"]);
        assert_eq!(f.entry(&tx(Actor::User, b"A")), ["→ 41"]);
    }

    #[test]
    fn sent_data_and_markers_end_the_pending_line() {
        let mut f = feed();
        f.entry(&rx(T0, b"uart:~$ "));
        assert_eq!(
            f.entry(&tx(Actor::User, b"led on\r\n")),
            ["uart:~$ ", "→ led on\\r\\n"]
        );
        assert_eq!(f.entry(&tx(Actor::Ai, b"\x01")), ["AI → \\x01"]);
        let marker = Entry {
            kind: EntryKind::Marker {
                label: "reconnected".into(),
            },
            ..rx(T0, b"")
        };
        assert_eq!(f.entry(&marker), ["# reconnected"]);
        assert_eq!(f.note("capture started"), ["· capture started"]);
        f.display.echo = false;
        assert!(f.entry(&tx(Actor::User, b"x")).is_empty());
        let mut colored = Feed::new("utf-8", 0, true).unwrap();
        assert_eq!(
            colored.entry(&tx(Actor::Ai, b"x")),
            ["\x1b[35mAI \x1b[0m\x1b[36m→ x\x1b[0m"]
        );
        assert_eq!(escape_controls("a\tb"), "a\\tb");
    }
}
