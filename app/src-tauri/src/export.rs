//! Saving a session's log to a file: a one-off export of what the log holds
//! now, or a live capture that keeps appending new entries.
//!
//! Two formats:
//! - `raw`: received bytes exactly as they arrived (sent data and markers
//!   are left out), e.g. to diff a device's output or keep a binary dump.
//! - `log`: one line per entry with local date and time, direction and
//!   content; control characters and bytes that are not valid UTF-8 are
//!   escaped (`\r`, `\n`, `\t`, `\\`, `\xNN`), so the file is lossless and
//!   still readable by people and AI tools.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serialx_core::{Cursor, Direction, Entry, EntryKind, Session, SessionState};

/// File format of an export or capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    /// Received bytes only, unchanged.
    Raw,
    /// One readable line per entry with timestamp and direction.
    Log,
}

/// Payload read from the session per step.
const READ_BYTES: usize = 1024 * 1024;
/// How often a capture looks for new entries.
const CAPTURE_POLL: Duration = Duration::from_millis(100);

/// Formats microseconds since the Unix epoch as local
/// `YYYY-MM-DD HH:MM:SS.uuuuuu`, `utc_offset_min` minutes east of UTC.
pub fn format_timestamp(timestamp_us: u64, utc_offset_min: i32) -> String {
    let local_us = i128::from(timestamp_us) + i128::from(utc_offset_min) * 60_000_000;
    let micros = local_us.rem_euclid(1_000_000);
    let secs = local_us.div_euclid(1_000_000);
    let day_secs = secs.rem_euclid(86_400);
    let days = secs.div_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}.{micros:06}",
        day_secs / 3600,
        day_secs % 3600 / 60,
        day_secs % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (H. Hinnant).
fn civil_from_days(days: i128) -> (i128, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i128::from(m <= 2);
    // The values are in range by construction (1..=31, 1..=12).
    (
        y,
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

/// Escapes bytes for a `log` line: valid UTF-8 text is kept, controls and
/// invalid bytes become `\r`, `\n`, `\t`, `\\` or `\xNN`.
pub fn escape(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        for c in chunk.valid().chars() {
            match c {
                '\r' => out.push_str("\\r"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\\' => out.push_str("\\\\"),
                c if c.is_control() => {
                    // Controls are single bytes below 0x80 or C1 (two bytes).
                    let mut buf = [0u8; 4];
                    for b in c.encode_utf8(&mut buf).bytes() {
                        let _ = write!(out, "\\x{b:02x}");
                    }
                }
                c => out.push(c),
            }
        }
        for b in chunk.invalid() {
            let _ = write!(out, "\\x{b:02x}");
        }
    }
    out
}

/// Bytes an entry contributes to a file of `format`.
pub fn render(entry: &Entry, format: ExportFormat, utc_offset_min: i32) -> Vec<u8> {
    match (format, &entry.kind) {
        (
            ExportFormat::Raw,
            EntryKind::Data {
                direction: Direction::Rx,
                bytes,
            },
        ) => bytes.to_vec(),
        (ExportFormat::Raw, _) => Vec::new(),
        (ExportFormat::Log, kind) => {
            let time = format_timestamp(entry.timestamp_us, utc_offset_min);
            let line = match kind {
                EntryKind::Data { direction, bytes } => {
                    let dir = match direction {
                        Direction::Rx => "RX",
                        Direction::Tx => "TX",
                    };
                    format!("{time} {dir} {}\n", escape(bytes))
                }
                EntryKind::Marker { label } => format!("{time} -- {}\n", escape(label.as_bytes())),
            };
            line.into_bytes()
        }
    }
}

/// Line noting entries that were dropped from the log before being saved.
fn missed_line(missed: u64, format: ExportFormat) -> Vec<u8> {
    match format {
        ExportFormat::Raw => Vec::new(),
        ExportFormat::Log => {
            format!("-- {missed} entries were dropped before they could be saved\n").into_bytes()
        }
    }
}

/// Appends entries from `cursor` to `out`; returns the next cursor and the
/// bytes written.
fn copy_new(
    session: &Session,
    cursor: Cursor,
    out: &mut impl Write,
    format: ExportFormat,
    utc_offset_min: i32,
) -> Result<(Cursor, u64), String> {
    let mut cursor = cursor;
    let mut written = 0u64;
    loop {
        let chunk = session
            .read(cursor, READ_BYTES)
            .map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        if chunk.missed > 0 {
            bytes.extend(missed_line(chunk.missed, format));
        }
        for entry in &chunk.entries {
            bytes.extend(render(entry, format, utc_offset_min));
        }
        out.write_all(&bytes).map_err(|e| e.to_string())?;
        written += bytes.len() as u64;
        let done = chunk.next == cursor || chunk.entries.is_empty();
        cursor = chunk.next;
        if done || cursor >= session.end() {
            return Ok((cursor, written));
        }
    }
}

fn create(path: &Path) -> Result<BufWriter<File>, String> {
    File::create(path)
        .map(BufWriter::new)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Result of an export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    /// Bytes written to the file.
    pub bytes: u64,
}

/// Writes everything the session's log holds now to `path` (replacing it).
pub fn export(
    session: &Session,
    path: &Path,
    format: ExportFormat,
    utc_offset_min: i32,
) -> Result<ExportSummary, String> {
    let mut out = create(path)?;
    let (_, bytes) = copy_new(session, Cursor(0), &mut out, format, utc_offset_min)?;
    out.flush().map_err(|e| e.to_string())?;
    Ok(ExportSummary { bytes })
}

/// State of a session's live capture.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStatus {
    /// Whether entries are still being saved.
    pub active: bool,
    /// The file being written.
    pub path: Option<String>,
    /// Bytes written so far.
    pub bytes: u64,
    /// Why the capture stopped early.
    pub error: Option<String>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Capture {
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<CaptureStatus>>,
    thread: Option<JoinHandle<()>>,
}

impl Capture {
    fn stop_and_join(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            // A panicked capture has nothing left to clean up.
            let _ = thread.join();
        }
    }
}

/// Live captures of all sessions.
#[derive(Default)]
pub struct Captures {
    jobs: Mutex<HashMap<u32, Capture>>,
}

impl std::fmt::Debug for Captures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Captures")
            .field("jobs", &lock(&self.jobs).len())
            .finish()
    }
}

impl Captures {
    /// Starts saving the session's log to `path` (replacing the file):
    /// everything already in the log, then new entries as they arrive,
    /// until stopped or the session ends. Replaces a previous capture.
    pub fn start(
        &self,
        id: u32,
        session: Arc<Session>,
        path: &Path,
        format: ExportFormat,
        utc_offset_min: i32,
    ) -> Result<CaptureStatus, String> {
        self.stop(id);
        let mut out = create(path)?;
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(CaptureStatus {
            active: true,
            path: Some(path.display().to_string()),
            bytes: 0,
            error: None,
        }));
        let thread = {
            let stop = Arc::clone(&stop);
            let status = Arc::clone(&status);
            std::thread::spawn(move || {
                let mut cursor = Cursor(0);
                loop {
                    let stopping = stop.load(Ordering::Relaxed);
                    let ended = session.state() != SessionState::Open;
                    let saved = copy_new(&session, cursor, &mut out, format, utc_offset_min)
                        .and_then(|r| out.flush().map(|()| r).map_err(|e| e.to_string()));
                    match saved {
                        Ok((next, bytes)) => {
                            cursor = next;
                            lock(&status).bytes += bytes;
                        }
                        Err(e) => {
                            lock(&status).error = Some(e);
                            break;
                        }
                    }
                    // Save what arrived before a stop or the end of the
                    // session, then finish.
                    if stopping || ended {
                        break;
                    }
                    std::thread::sleep(CAPTURE_POLL);
                }
                lock(&status).active = false;
            })
        };
        let snapshot = lock(&status).clone();
        lock(&self.jobs).insert(
            id,
            Capture {
                stop,
                status,
                thread: Some(thread),
            },
        );
        Ok(snapshot)
    }

    /// Stops the session's capture after saving what has arrived; the
    /// final status stays readable until the next start.
    pub fn stop(&self, id: u32) {
        let capture = lock(&self.jobs).remove(&id);
        if let Some(capture) = capture {
            let status = Arc::clone(&capture.status);
            capture.stop_and_join();
            // Keep the final numbers visible.
            let finished = lock(&status).clone();
            lock(&self.jobs).insert(
                id,
                Capture {
                    stop: Arc::new(AtomicBool::new(true)),
                    status: Arc::new(Mutex::new(finished)),
                    thread: None,
                },
            );
        }
    }

    /// State of the session's capture (inactive and empty if none).
    pub fn status(&self, id: u32) -> CaptureStatus {
        lock(&self.jobs)
            .get(&id)
            .map(|c| lock(&c.status).clone())
            .unwrap_or_default()
    }

    /// Stops the capture and forgets it (when the session is closed).
    pub fn remove(&self, id: u32) {
        let capture = lock(&self.jobs).remove(&id);
        if let Some(capture) = capture {
            capture.stop_and_join();
        }
    }

    /// Stops every capture (on app exit).
    pub fn stop_all(&self) {
        let jobs: Vec<Capture> = lock(&self.jobs).drain().map(|(_, c)| c).collect();
        for capture in jobs {
            capture.stop_and_join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use serialx_core::transport::mock::{self, MockDevice};
    use serialx_core::{ManualClock, SessionId, SessionOptions};

    use super::*;
    use crate::settings::tests::TempDir;

    /// 2026-11-18 10:13:38.123456 UTC.
    const T0: u64 = 1_794_996_818_123_456;

    fn session() -> (Arc<Session>, MockDevice, Arc<ManualClock>) {
        let (connection, device) = mock::connection("mock0", Duration::from_millis(5));
        let clock = Arc::new(ManualClock::new(T0));
        let session = Session::start(
            SessionId(1),
            connection,
            Arc::clone(&clock) as Arc<dyn serialx_core::Clock>,
            SessionOptions::default(),
        )
        .unwrap();
        (Arc::new(session), device, clock)
    }

    fn wait_until(check: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !check() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn timestamps_are_local_with_microseconds() {
        assert_eq!(format_timestamp(T0, 0), "2026-11-18 10:13:38.123456");
        assert_eq!(format_timestamp(T0, 480), "2026-11-18 18:13:38.123456");
        assert_eq!(format_timestamp(T0, -660), "2026-11-17 23:13:38.123456");
        assert_eq!(format_timestamp(0, 0), "1970-01-01 00:00:00.000000");
        assert_eq!(format_timestamp(0, -60), "1969-12-31 23:00:00.000000");
        // A leap day.
        assert_eq!(
            format_timestamp(1_709_164_800_000_000, 0),
            "2024-02-29 00:00:00.000000"
        );
    }

    #[test]
    fn escaping_is_lossless_and_readable() {
        assert_eq!(escape(b"OK\r\n"), "OK\\r\\n");
        assert_eq!(escape("温度 23\tC".as_bytes()), "温度 23\\tC");
        assert_eq!(escape(b"\x1b[32mA\\"), "\\x1b[32mA\\\\");
        assert_eq!(escape(&[0xff, 0x41, 0x00]), "\\xffA\\x00");
        assert_eq!(escape("\u{85}".as_bytes()), "\\xc2\\x85");
    }

    #[test]
    fn exports_raw_and_log_formats() {
        let (s, device, clock) = session();
        s.mark("reset");
        clock.advance(1_000);
        s.write(b"AT\r\n").unwrap();
        device.send(b"OK\r\n\xff");
        wait_until(|| s.end() == Cursor(3));
        let dir = TempDir::new("export");

        let raw = dir.0.join("out.bin");
        let summary = export(&s, &raw, ExportFormat::Raw, 0).unwrap();
        assert_eq!(std::fs::read(&raw).unwrap(), b"OK\r\n\xff");
        assert_eq!(summary.bytes, 5);

        let log = dir.0.join("out.log");
        export(&s, &log, ExportFormat::Log, 480).unwrap();
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "2026-11-18 18:13:38.123456 -- reset\n\
             2026-11-18 18:13:38.124456 TX AT\\r\\n\n\
             2026-11-18 18:13:38.124456 RX OK\\r\\n\\xff\n"
        );
        assert!(
            export(&s, &dir.0.join("missing/dir/x.log"), ExportFormat::Log, 0)
                .unwrap_err()
                .contains("cannot write")
        );
    }

    #[test]
    fn exports_large_logs_in_steps() {
        let (s, device, _clock) = session();
        let chunk = vec![b'x'; 300 * 1024];
        for _ in 0..5 {
            device.send(&chunk);
        }
        wait_until(|| {
            s.read(Cursor(0), usize::MAX)
                .unwrap()
                .entries
                .iter()
                .map(|e| match &e.kind {
                    EntryKind::Data { bytes, .. } => bytes.len(),
                    EntryKind::Marker { .. } => 0,
                })
                .sum::<usize>()
                == 5 * chunk.len()
        });
        let dir = TempDir::new("export-large");
        let path = dir.0.join("big.bin");
        assert_eq!(
            export(&s, &path, ExportFormat::Raw, 0).unwrap().bytes,
            5 * 300 * 1024
        );
    }

    #[test]
    fn captures_history_then_new_data_until_stopped() {
        let (s, device, _clock) = session();
        device.send(b"before\n");
        wait_until(|| s.end() == Cursor(1));
        let dir = TempDir::new("capture");
        let path = dir.0.join("live.bin");
        let captures = Captures::default();
        assert_eq!(captures.status(1), CaptureStatus::default());
        let started = captures
            .start(1, Arc::clone(&s), &path, ExportFormat::Raw, 0)
            .unwrap();
        assert!(started.active);
        assert_eq!(
            started.path.as_deref(),
            Some(path.display().to_string().as_str())
        );
        device.send(b"after\n");
        wait_until(|| captures.status(1).bytes == 13);
        captures.stop(1);
        let done = captures.status(1);
        assert!(!done.active);
        assert_eq!(done.bytes, 13, "the final numbers stay visible");
        assert_eq!(std::fs::read(&path).unwrap(), b"before\nafter\n");
        device.send(b"ignored\n");
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(std::fs::read(&path).unwrap(), b"before\nafter\n");
        captures.remove(1);
        assert_eq!(captures.status(1), CaptureStatus::default());
    }

    #[test]
    fn capture_ends_with_the_session_and_reports_errors() {
        let (s, device, _clock) = session();
        let dir = TempDir::new("capture-end");
        let captures = Captures::default();
        captures
            .start(
                1,
                Arc::clone(&s),
                &dir.0.join("a.log"),
                ExportFormat::Log,
                0,
            )
            .unwrap();
        device.send(b"last\n");
        device.disconnect();
        wait_until(|| !captures.status(1).active);
        let text = std::fs::read_to_string(dir.0.join("a.log")).unwrap();
        assert!(text.contains("RX last\\n"), "{text}");
        assert!(
            captures
                .start(1, s, &dir.0.join("no/such/dir.log"), ExportFormat::Log, 0)
                .unwrap_err()
                .contains("cannot write")
        );
        assert!(format!("{captures:?}").contains("Captures"));
        captures.stop_all();
    }

    #[test]
    fn dropped_entries_are_noted_in_logs() {
        let (s, device, _clock) = session();
        let options = SessionOptions {
            log_capacity: 200,
            ..SessionOptions::default()
        };
        let (connection, small_device) = mock::connection("mock1", Duration::from_millis(5));
        let small = Session::start(
            SessionId(2),
            connection,
            Arc::new(ManualClock::new(T0)),
            options,
        )
        .unwrap();
        for i in 0..20 {
            small_device.send(format!("line {i:02}\n").as_bytes());
            std::thread::sleep(Duration::from_millis(2));
        }
        wait_until(|| small.end().0 >= 5);
        let dir = TempDir::new("missed");
        let path = dir.0.join("m.log");
        export(&small, &path, ExportFormat::Log, 0).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("-- "), "{text}");
        assert!(text.contains("entries were dropped"));
        assert!(missed_line(3, ExportFormat::Raw).is_empty());
        drop((s, device));
    }
}
