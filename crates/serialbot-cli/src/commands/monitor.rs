// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! `monitor`: only receive, streaming to standard output until Ctrl+C,
//! `--until` matches or `--duration` ends.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::json;
use serialbot_core::log::{Cursor, Direction, Entry, EntryKind};
use serialbot_core::session::{Session, SessionState};
use serialbot_core::{SessionManager, hex};
use serialbot_jobs::export::{Captures, ExportFormat, format_timestamp};

use crate::Failure;
use crate::args::MonitorArgs;
use crate::commands::send::open;
use crate::exit::Exit;
use crate::output::{Output, Style};

/// How often new entries are looked for.
const POLL: Duration = Duration::from_millis(20);

/// Bytes read from the log per step.
const READ_STEP: usize = 256 * 1024;

/// How many trailing received bytes `--until` may span.
const UNTIL_WINDOW: usize = 64 * 1024;

/// What stopped the monitor.
#[derive(Debug, PartialEq, Eq)]
enum Stop {
    Interrupted,
    Matched,
    TimeUp,
    Lost(SessionState),
}

/// Streams what the device sends. Exit codes: 0 on Ctrl+C, when `--until`
/// matched, or when `--duration` ends without `--until`; 1 when
/// `--duration` ends before `--until` matched; 3 the port cannot be
/// opened; 4 the device went away. `--log` also saves everything to a file
/// (as the app's "log with timestamps" format) until the end.
pub fn run(
    args: &MonitorArgs,
    manager: &SessionManager,
    interrupted: &Arc<AtomicBool>,
    utc_offset_min: i32,
    out: &mut Output<'_>,
) -> Result<(), Failure> {
    let s = out.strings();
    let session = open(manager, &args.port, s)?;
    let captures = Captures::default();
    if let Some(path) = &args.log
        && let Err(e) = captures.start(1, &session, path, ExportFormat::Log, utc_offset_min)
    {
        let _ = manager.remove(session.id());
        return Err(Failure::new(Exit::Usage, e));
    }
    let mut view = View {
        args,
        utc_offset_min,
        at_line_start: true,
        tail: Vec::new(),
    };
    let stop = view.stream(&session, interrupted, out);
    captures.stop_all();
    let _ = manager.remove(session.id());
    match stop {
        Stop::Interrupted | Stop::Matched => Ok(()),
        Stop::TimeUp if args.until.is_none() => Ok(()),
        Stop::TimeUp => Err(Failure::new(Exit::Timeout, s.no_match.to_owned())),
        Stop::Lost(state) => Err(Failure::new(
            Exit::Disconnected,
            format!("{} ({}): {state:?}", s.disconnected, args.port.port),
        )),
    }
}

struct View<'a> {
    args: &'a MonitorArgs,
    utc_offset_min: i32,
    /// Whether the next text byte starts a line (for `--time`).
    at_line_start: bool,
    /// The last received bytes, for `--until`.
    tail: Vec<u8>,
}

impl View<'_> {
    fn stream(
        &mut self,
        session: &Session,
        interrupted: &AtomicBool,
        out: &mut Output<'_>,
    ) -> Stop {
        let deadline = self.args.duration.map(|d| Instant::now() + d);
        let mut cursor = Cursor(0);
        loop {
            // Read before looking at the state, so data that arrived just
            // before an unplug is still shown.
            let state = session.state();
            while let Ok(chunk) = session.read(cursor, READ_STEP) {
                if chunk.entries.is_empty() {
                    break;
                }
                for entry in &chunk.entries {
                    if self.show(entry, out) {
                        return Stop::Matched;
                    }
                }
                cursor = chunk.next;
            }
            if state != SessionState::Open {
                return Stop::Lost(state);
            }
            if interrupted.load(Ordering::Relaxed) {
                return Stop::Interrupted;
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                return Stop::TimeUp;
            }
            std::thread::sleep(POLL);
        }
    }

    /// Prints one entry; true when `--until` matched.
    fn show(&mut self, entry: &Entry, out: &mut Output<'_>) -> bool {
        let (kind, bytes) = match &entry.kind {
            EntryKind::Data {
                direction: Direction::Rx,
                bytes,
            } => ("rx", bytes.as_ref()),
            EntryKind::Marker { label } if self.args.json => {
                out.json(&json!({
                    "seq": entry.seq,
                    "timestamp_us": entry.timestamp_us,
                    "kind": "marker",
                    "data": label,
                    "ai": false,
                }));
                return false;
            }
            _ => return false,
        };
        if self.args.json {
            let data = if self.args.hex {
                hex::to_hex(bytes)
            } else {
                String::from_utf8_lossy(bytes).into_owned()
            };
            out.json(&json!({
                "seq": entry.seq,
                "timestamp_us": entry.timestamp_us,
                "kind": kind,
                "data": data,
                "ai": false,
            }));
        } else if self.args.hex {
            let line = format!("{}{}", self.stamp(entry, out), hex::to_hex(bytes));
            out.line(&line);
        } else {
            self.text(entry, bytes, out);
        }
        self.matched(bytes)
    }

    /// `[HH:MM:SS.mmm] ` in local time with `--time`, else nothing.
    fn stamp(&self, entry: &Entry, out: &Output<'_>) -> String {
        if !self.args.time {
            return String::new();
        }
        let full = format_timestamp(entry.timestamp_us, self.utc_offset_min);
        let time = full.get(11..23).unwrap_or(&full);
        format!("{} ", out.paint(Style::Dim, &format!("[{time}]")))
    }

    /// Text unchanged; with `--time` each new line starts with a stamp.
    fn text(&mut self, entry: &Entry, bytes: &[u8], out: &mut Output<'_>) {
        if !self.args.time {
            out.bytes(bytes);
            return;
        }
        let stamp = self.stamp(entry, out);
        for piece in bytes.split_inclusive(|b| *b == b'\n') {
            if self.at_line_start {
                out.bytes(stamp.as_bytes());
            }
            out.bytes(piece);
            self.at_line_start = piece.ends_with(b"\n");
        }
    }

    /// Adds `bytes` to the tail and checks `--until` against it.
    fn matched(&mut self, bytes: &[u8]) -> bool {
        let Some(pattern) = &self.args.until else {
            return false;
        };
        self.tail.extend_from_slice(bytes);
        let excess = self.tail.len().saturating_sub(UNTIL_WINDOW);
        self.tail.drain(..excess);
        pattern.is_match(&self.tail)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;
    use std::time::Duration;

    use crate::exit::Exit;
    use crate::testing::Fixture;

    /// Sends `chunks` from the device on `port` once it is open, one every
    /// few milliseconds.
    fn feed(f: &Fixture, chunks: &'static [&'static [u8]]) -> std::thread::JoinHandle<()> {
        let devices = std::sync::Arc::clone(&f.devices);
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let device = loop {
                if let Some(d) = devices.lock().unwrap().get("mock0").cloned() {
                    break d;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            };
            for chunk in chunks {
                device.send(chunk);
                std::thread::sleep(Duration::from_millis(5));
            }
        })
    }

    #[test]
    fn streams_until_the_pattern_matches_across_chunks() {
        let f = Fixture::new();
        let device = feed(&f, &[b"boot\r\nrea", b"dy> ", b"after"]);
        let ran = f.run(&["monitor", "mock0", "--until", "ready>", "--duration", "5s"]);
        device.join().unwrap();
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert_eq!(ran.out, "boot\r\nready> ", "stops at the matching entry");
        assert!(f.manager.list().is_empty(), "the port is closed again");
    }

    #[test]
    fn the_duration_ends_it_and_fails_only_with_until() {
        let f = Fixture::new();
        let device = feed(&f, &[b"tick\n"]);
        let ran = f.run(&["monitor", "mock0", "--duration", "300ms"]);
        device.join().unwrap();
        assert_eq!((ran.code, ran.out.as_str()), (0, "tick\n"));
        let ran = f.run(&[
            "monitor",
            "mock0",
            "--duration",
            "100ms",
            "--until",
            "never",
        ]);
        assert_eq!(ran.code, Exit::Timeout as u8);
        assert!(ran.err.contains("no reply matched"), "{}", ran.err);
    }

    #[test]
    fn ctrl_c_ends_it_successfully() {
        let f = Fixture::new();
        let interrupted = std::sync::Arc::clone(&f.interrupted);
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            interrupted.store(true, Ordering::Relaxed);
        });
        let ran = f.run(&["monitor", "mock0"]);
        stopper.join().unwrap();
        assert_eq!(ran.code, 0, "{}", ran.err);
    }

    #[test]
    fn time_stamps_each_line_and_hex_shows_bytes() {
        let f = Fixture::new();
        let device = feed(&f, &[b"a\nb", b"c\n"]);
        let ran = f.run(&[
            "monitor",
            "mock0",
            "--time",
            "--until",
            "c\\n",
            "--duration",
            "5s",
        ]);
        device.join().unwrap();
        assert_eq!(ran.code, 0, "{}", ran.err);
        let lines: Vec<&str> = ran.out.lines().collect();
        assert_eq!(lines.len(), 2, "{:?}", ran.out);
        for (line, text) in lines.iter().zip(["a", "bc"]) {
            assert!(line.starts_with('['), "{line}");
            assert_eq!(&line[13..], format!("] {text}"), "{line}");
        }
        f.devices.lock().unwrap().clear();
        let device = feed(&f, &[b"\x01\xff"]);
        // `(?-u)` makes `\xff` the byte, not the character U+00FF.
        let ran = f.run(&[
            "monitor",
            "mock0",
            "--hex",
            "--until",
            "(?-u)\\xff",
            "--duration",
            "5s",
        ]);
        device.join().unwrap();
        assert_eq!((ran.code, ran.out.as_str()), (0, "01 ff\n"));
    }

    #[test]
    fn json_lines_have_the_mcp_fields() {
        let f = Fixture::new();
        let device = feed(&f, &[b"OK\r\n"]);
        let ran = f.run(&[
            "monitor",
            "mock0",
            "--json",
            "--until",
            "OK",
            "--duration",
            "5s",
        ]);
        device.join().unwrap();
        let rows: Vec<serde_json::Value> = ran
            .out
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let rx: Vec<_> = rows.iter().filter(|r| r["kind"] == "rx").collect();
        assert_eq!(rx.len(), 1, "{}", ran.out);
        assert_eq!(rx[0]["data"], "OK\r\n");
        assert_eq!(rx[0]["ai"], false);
        assert!(rx[0]["timestamp_us"].as_u64().unwrap() > 0);
        f.devices.lock().unwrap().clear();
        let device = feed(&f, &[b"\x00"]);
        let ran = f.run(&[
            "monitor",
            "mock0",
            "--json",
            "--hex",
            "--until",
            "\\x00",
            "--duration",
            "5s",
        ]);
        device.join().unwrap();
        assert!(ran.out.contains("\"data\":\"00\""), "{}", ran.out);
    }

    #[test]
    fn a_lost_device_exits_with_4_after_showing_its_last_data() {
        let f = Fixture::new();
        let devices = std::sync::Arc::clone(&f.devices);
        let unplug = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(d) = devices.lock().unwrap().get("mock0") {
                    d.send(b"bye");
                    d.disconnect();
                    return;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        let ran = f.run(&["monitor", "mock0", "--duration", "5s"]);
        unplug.join().unwrap();
        assert_eq!(ran.code, Exit::Disconnected as u8, "{}", ran.err);
        assert_eq!(ran.out, "bye");
        assert!(
            ran.err.contains("the device went away (mock0)"),
            "{}",
            ran.err
        );
    }

    #[test]
    fn log_saves_to_a_file_as_well() {
        let f = Fixture::new();
        let dir =
            std::env::temp_dir().join(format!("serialbot-cli-monitor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.log");
        let device = feed(&f, &[b"hello\n"]);
        let ran = f.run(&[
            "monitor",
            "mock0",
            "--log",
            path.to_str().unwrap(),
            "--until",
            "hello",
            "--duration",
            "5s",
        ]);
        device.join().unwrap();
        assert_eq!(ran.code, 0, "{}", ran.err);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(" RX hello\\n"), "{text}");
        let bad = f.run(&[
            "monitor",
            "mock0",
            "--log",
            "/no/such/dir/x.log",
            "--duration",
            "10ms",
        ]);
        assert_eq!(bad.code, Exit::Usage as u8, "{}", bad.err);
        assert!(f.manager.list().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
