//! A session owns one open connection.
//!
//! A dedicated reader thread moves every received chunk into the session's
//! [`LogBuffer`]; writes go straight to the transport and are logged as
//! well. The UI, scripts and MCP tools are all readers of the same log, so
//! they see exactly the same bytes and timestamps.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bytes::Bytes;
use regex::bytes::Regex;

use crate::clock::Clock;
use crate::log::{Cursor, Direction, EntryKind, LogBuffer, LogError, ReadResult};
use crate::transport::{
    Connection, LineStates, TransportControl, TransportError, TransportInfo, TransportReader,
};

/// Identifier of a session, unique within one [`crate::SessionManager`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionId(pub u32);

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "s{}", self.0)
    }
}

/// Lifecycle state of a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    /// The connection is open and being read.
    Open,
    /// The device went away; the log is kept.
    Disconnected,
    /// Reading failed with an error other than a disconnect.
    Failed(String),
    /// The session was closed on request.
    Closed,
}

/// Errors returned by session operations.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// The transport failed.
    #[error(transparent)]
    Transport(#[from] TransportError),
    /// A read used an invalid cursor.
    #[error(transparent)]
    Log(#[from] LogError),
    /// The session is no longer open.
    #[error("session is {0:?}")]
    NotOpen(SessionState),
}

/// Tunable session settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionOptions {
    /// Byte budget of the session log.
    pub log_capacity: usize,
    /// Size of the reader thread's buffer, i.e. the largest single chunk.
    pub read_chunk: usize,
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            log_capacity: 64 * 1024 * 1024,
            read_chunk: 64 * 1024,
        }
    }
}

/// A successful [`Session::wait_for`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitMatch {
    /// The bytes the pattern matched.
    pub matched: Vec<u8>,
    /// Sequence number of the entry where the match ends.
    pub seq: u64,
    /// Cursor just past that entry; pass it to the next wait or read.
    pub next: Cursor,
}

struct Inner {
    log: LogBuffer,
    state: SessionState,
}

struct Shared {
    inner: Mutex<Inner>,
    changed: Condvar,
    stop: AtomicBool,
    clock: Arc<dyn Clock>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sets a terminal state unless one is already set, and wakes waiters.
    fn finish(&self, state: SessionState) {
        let mut inner = self.lock();
        if inner.state == SessionState::Open {
            inner.state = state;
        }
        drop(inner);
        self.changed.notify_all();
    }
}

/// Levels last driven on the output lines; `None` until first set in this
/// session (the level the OS chose at open is not known).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OutputSignals {
    /// Data Terminal Ready.
    pub dtr: Option<bool>,
    /// Request To Send.
    pub rts: Option<bool>,
}

/// One open connection with its log and reader thread.
pub struct Session {
    id: SessionId,
    alias: Mutex<Option<String>>,
    outputs: Mutex<OutputSignals>,
    info: TransportInfo,
    shared: Arc<Shared>,
    control: Mutex<Box<dyn TransportControl>>,
    reader: Mutex<Option<JoinHandle<()>>>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("info", &self.info)
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Starts a session over `connection` and spawns its reader thread.
    pub fn start(
        id: SessionId,
        connection: Connection,
        clock: Arc<dyn Clock>,
        options: SessionOptions,
    ) -> Result<Self, SessionError> {
        let shared = Arc::new(Shared {
            inner: Mutex::new(Inner {
                log: LogBuffer::new(options.log_capacity)?,
                state: SessionState::Open,
            }),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
            clock,
        });
        let thread_shared = Arc::clone(&shared);
        let reader = connection.reader;
        let chunk = options.read_chunk.max(1);
        let handle = std::thread::Builder::new()
            .name(format!("serialx-rx-{id}"))
            .spawn(move || read_loop(&thread_shared, reader, chunk))
            .map_err(TransportError::Io)?;
        Ok(Self {
            id,
            alias: Mutex::new(None),
            outputs: Mutex::new(OutputSignals::default()),
            info: connection.info,
            shared,
            control: Mutex::new(connection.control),
            reader: Mutex::new(Some(handle)),
        })
    }

    /// Session identifier.
    pub fn id(&self) -> SessionId {
        self.id
    }

    /// Description of the underlying connection.
    pub fn info(&self) -> &TransportInfo {
        &self.info
    }

    /// User-chosen name such as "main board", if any.
    pub fn alias(&self) -> Option<String> {
        lock(&self.alias).clone()
    }

    /// Sets or clears the user-chosen name.
    pub fn set_alias(&self, alias: Option<String>) {
        *lock(&self.alias) = alias;
    }

    /// Current lifecycle state.
    pub fn state(&self) -> SessionState {
        self.shared.lock().state.clone()
    }

    /// Cursor just past the newest log entry.
    pub fn end(&self) -> Cursor {
        self.shared.lock().log.end()
    }

    /// Reads log entries from `cursor`; see [`LogBuffer::read`].
    pub fn read(&self, cursor: Cursor, max_payload: usize) -> Result<ReadResult, SessionError> {
        Ok(self.shared.lock().log.read(cursor, max_payload)?)
    }

    /// Inserts a marker (for example "reset") and returns its sequence
    /// number. Works in any state so markers can annotate a dead session.
    pub fn mark(&self, label: &str) -> u64 {
        let now = self.shared.clock.now_us();
        let seq = self.shared.lock().log.push_marker(now, label);
        self.shared.changed.notify_all();
        seq
    }

    /// Sends `data` and logs it; returns the sequence number of the entry.
    ///
    /// The TX entry is logged *before* the bytes are written, stamped with
    /// the time sending started, so a reply that arrives while the write is
    /// still in progress always follows its command in the log. If the
    /// write then fails, a `TX failed: …` marker is logged after the entry
    /// and the error is returned.
    pub fn write(&self, data: &[u8]) -> Result<u64, SessionError> {
        self.ensure_open()?;
        // Holding the control lock keeps concurrent writers' entries in the
        // order their bytes go out.
        let mut control = lock(&self.control);
        let now = self.shared.clock.now_us();
        let seq =
            self.shared
                .lock()
                .log
                .push_data(now, Direction::Tx, Bytes::copy_from_slice(data));
        self.shared.changed.notify_all();
        if let Err(e) = control.write_all(data) {
            drop(control);
            self.mark(&format!("TX failed: {e}"));
            return Err(e.into());
        }
        Ok(seq)
    }

    /// Drives DTR. On success the level is remembered and a `DTR=0`/`DTR=1`
    /// marker is logged, so resets are visible in the timeline.
    pub fn set_dtr(&self, level: bool) -> Result<(), SessionError> {
        self.with_control(|c| c.set_dtr(level))?;
        lock(&self.outputs).dtr = Some(level);
        self.mark(&format!("DTR={}", u8::from(level)));
        Ok(())
    }

    /// Drives RTS; remembered and logged like [`Session::set_dtr`].
    ///
    /// Refused with [`TransportError::Unsupported`] while hardware flow
    /// control owns RTS (see [`Capabilities::manual_rts`]); the driver is not
    /// asked.
    ///
    /// [`Capabilities::manual_rts`]: crate::transport::Capabilities::manual_rts
    pub fn set_rts(&self, level: bool) -> Result<(), SessionError> {
        if !self.info.capabilities.manual_rts {
            return Err(SessionError::Transport(TransportError::Unsupported(
                "setting RTS while RTS/CTS hardware flow control is on",
            )));
        }
        self.with_control(|c| c.set_rts(level))?;
        lock(&self.outputs).rts = Some(level);
        self.mark(&format!("RTS={}", u8::from(level)));
        Ok(())
    }

    /// Levels last set with [`Session::set_dtr`] / [`Session::set_rts`].
    pub fn output_signals(&self) -> OutputSignals {
        *lock(&self.outputs)
    }

    /// Reads CTS/DSR/RI/CD.
    pub fn line_states(&self) -> Result<LineStates, SessionError> {
        self.with_control(|c| c.line_states())
    }

    /// Holds a BREAK for `duration`.
    pub fn send_break(&self, duration: Duration) -> Result<(), SessionError> {
        self.with_control(|c| c.send_break(duration))
    }

    /// Changes the baud rate without closing the port.
    pub fn set_baud_rate(&self, baud_rate: u32) -> Result<(), SessionError> {
        self.with_control(|c| c.set_baud_rate(baud_rate))
    }

    /// Waits until received bytes after `from` match `pattern`.
    ///
    /// Only received (RX) data is searched; the bytes of consecutive chunks
    /// are concatenated, so a match may span chunks, within the last
    /// `match_window` bytes. Returns `Ok(None)` on
    /// timeout. If the session stops being open while waiting, data already
    /// received is still searched and then [`SessionError::NotOpen`] is
    /// returned.
    pub fn wait_for(
        &self,
        pattern: &Regex,
        from: Cursor,
        timeout: Duration,
        match_window: usize,
    ) -> Result<Option<WaitMatch>, SessionError> {
        let deadline = Instant::now() + timeout;
        let mut window = MatchWindow::new(match_window.max(1));
        let mut cursor = from;
        let mut inner = self.shared.lock();
        loop {
            let batch = inner.log.read(cursor, usize::MAX)?;
            cursor = batch.next;
            for entry in &batch.entries {
                if let EntryKind::Data {
                    direction: Direction::Rx,
                    bytes,
                } = &entry.kind
                {
                    window.push(entry.seq, bytes);
                }
            }
            if let Some(found) = window.find(pattern) {
                return Ok(Some(found));
            }
            if inner.state != SessionState::Open {
                return Err(SessionError::NotOpen(inner.state.clone()));
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            inner = self
                .shared
                .changed
                .wait_timeout(inner, deadline - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Sends `data`, then waits for `pattern` in what is received from the
    /// moment of sending (including replies that arrive before the write
    /// call returns).
    pub fn send_and_expect(
        &self,
        data: &[u8],
        pattern: &Regex,
        timeout: Duration,
        match_window: usize,
    ) -> Result<Option<WaitMatch>, SessionError> {
        // Take the cursor *before* writing: a fast device can answer before
        // the TX entry is logged, giving the reply a smaller sequence number
        // than the command. Only RX data is matched, so starting here never
        // matches the command itself.
        let from = self.end();
        self.write(data)?;
        self.wait_for(pattern, from, timeout, match_window)
    }

    /// Stops the reader thread and closes the port. Idempotent; the log
    /// stays readable afterwards.
    pub fn close(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = lock(&self.reader).take() {
            // The reader returns within one read timeout. A panicked reader
            // has nothing left to clean up.
            let _ = handle.join();
        }
        self.shared.finish(SessionState::Closed);
    }

    fn ensure_open(&self) -> Result<(), SessionError> {
        match self.state() {
            SessionState::Open => Ok(()),
            other => Err(SessionError::NotOpen(other)),
        }
    }

    fn with_control<T>(
        &self,
        f: impl FnOnce(&mut dyn TransportControl) -> Result<T, TransportError>,
    ) -> Result<T, SessionError> {
        self.ensure_open()?;
        Ok(f(lock(&self.control).as_mut())?)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn read_loop(shared: &Shared, mut reader: Box<dyn TransportReader>, chunk: usize) {
    let mut buf = vec![0u8; chunk];
    while !shared.stop.load(Ordering::SeqCst) {
        match reader.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                let now = shared.clock.now_us();
                shared
                    .lock()
                    .log
                    .push_data(now, Direction::Rx, Bytes::copy_from_slice(&buf[..n]));
                shared.changed.notify_all();
            }
            Err(TransportError::Disconnected) => {
                shared.finish(SessionState::Disconnected);
                return;
            }
            Err(e) => {
                shared.finish(SessionState::Failed(e.to_string()));
                return;
            }
        }
    }
}

/// Recent received bytes with the sequence number each byte came from.
struct MatchWindow {
    bytes: Vec<u8>,
    /// `(offset in bytes, seq)` for the start of every chunk in `bytes`.
    starts: Vec<(usize, u64)>,
    limit: usize,
}

impl MatchWindow {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            starts: Vec::new(),
            limit,
        }
    }

    fn push(&mut self, seq: u64, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        self.starts.push((self.bytes.len(), seq));
        self.bytes.extend_from_slice(data);
        self.trim();
    }

    /// Drops the oldest bytes beyond the limit, keeping offsets consistent.
    fn trim(&mut self) {
        let excess = self.bytes.len().saturating_sub(self.limit);
        if excess == 0 {
            return;
        }
        self.bytes.drain(..excess);
        // Chunks that end before the cut disappear; the first surviving
        // chunk is clamped to start at offset 0.
        let first_kept = self
            .starts
            .iter()
            .rposition(|&(off, _)| off <= excess)
            .unwrap_or(0);
        self.starts.drain(..first_kept);
        for start in &mut self.starts {
            start.0 = start.0.saturating_sub(excess);
        }
    }

    fn find(&self, pattern: &Regex) -> Option<WaitMatch> {
        let m = pattern.find(&self.bytes)?;
        // The entry containing the last matched byte (or the match start
        // for an empty match).
        let last = m.end().saturating_sub(1).max(m.start());
        let idx = self.starts.partition_point(|&(off, _)| off <= last);
        let seq = self.starts[idx.saturating_sub(1)].1;
        Some(WaitMatch {
            matched: m.as_bytes().to_vec(),
            seq,
            next: Cursor(seq + 1),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crate::transport::mock::{self, MockDevice};

    const T: Duration = Duration::from_millis(5);
    const LONG: Duration = Duration::from_secs(5);
    const WIN: usize = 64 * 1024;

    fn session() -> (Session, MockDevice, Arc<ManualClock>) {
        let (conn, dev) = mock::connection("mock0", T);
        let clock = Arc::new(ManualClock::new(1_000));
        let s =
            Session::start(SessionId(1), conn, clock.clone(), SessionOptions::default()).unwrap();
        (s, dev, clock)
    }

    fn re(p: &str) -> Regex {
        Regex::new(p).unwrap()
    }

    fn rx_bytes(s: &Session) -> Vec<u8> {
        let r = s.read(Cursor(0), usize::MAX).unwrap();
        r.entries
            .iter()
            .filter_map(|e| match &e.kind {
                EntryKind::Data {
                    direction: Direction::Rx,
                    bytes,
                } => Some(bytes.to_vec()),
                _ => None,
            })
            .flatten()
            .collect()
    }

    #[test]
    fn id_displays_with_prefix_and_debug_shows_state() {
        let (s, _d, _c) = session();
        assert_eq!(s.id().to_string(), "s1");
        assert_eq!(s.info().name, "mock0");
        assert!(format!("{s:?}").contains("Open"));
    }

    #[test]
    fn received_bytes_land_in_the_log_with_timestamps() {
        let (s, d, _c) = session();
        d.send(b"boot ok\r\n");
        let m = s.wait_for(&re("ok"), Cursor(0), LONG, WIN).unwrap();
        assert!(m.is_some());
        let r = s.read(Cursor(0), usize::MAX).unwrap();
        assert_eq!(r.entries[0].timestamp_us, 1_000);
        assert_eq!(rx_bytes(&s), b"boot ok\r\n");
    }

    #[test]
    fn writes_reach_the_device_and_are_logged_as_tx() {
        let (s, d, clock) = session();
        clock.advance(42);
        let seq = s.write(b"AT\r\n").unwrap();
        assert_eq!(d.take_written(), b"AT\r\n");
        let r = s.read(Cursor(seq), usize::MAX).unwrap();
        assert_eq!(r.entries[0].timestamp_us, 1_042);
        assert_eq!(
            r.entries[0].kind,
            EntryKind::Data {
                direction: Direction::Tx,
                bytes: Bytes::from_static(b"AT\r\n"),
            }
        );
    }

    #[test]
    fn wait_for_matches_across_chunk_boundaries() {
        let (s, d, _c) = session();
        let from = s.end();
        // Make sure the two halves become separate log entries: a reader
        // that finds both queued would merge them into one chunk.
        d.send(b"rst:0x1 (POWERON),boot:0x13 (SPI_FA");
        s.wait_for(&re("SPI_FA"), from, LONG, WIN).unwrap().unwrap();
        d.send(b"ST_FLASH_BOOT)\r\nready");
        let m = s
            .wait_for(&re(r"SPI_FAST_FLASH_BOOT"), from, LONG, WIN)
            .unwrap()
            .unwrap();
        assert_eq!(m.matched, b"SPI_FAST_FLASH_BOOT");
        let entries = s.read(from, usize::MAX).unwrap().entries;
        assert_eq!(entries.len(), 2, "the halves were logged separately");
        assert_eq!(m.seq, entries[1].seq, "the match ends in the second chunk");
    }

    #[test]
    fn wait_for_ignores_sent_bytes_and_data_before_the_cursor() {
        let (s, d, _c) = session();
        d.send(b"OK old\r\n");
        assert!(
            s.wait_for(&re("old"), Cursor(0), LONG, WIN)
                .unwrap()
                .is_some()
        );
        let from = s.end();
        s.write(b"OK echo of tx").unwrap();
        assert_eq!(
            s.wait_for(&re("OK"), from, Duration::from_millis(30), WIN)
                .unwrap(),
            None
        );
    }

    #[test]
    fn wait_for_returns_none_on_timeout() {
        let (s, _d, _c) = session();
        let started = Instant::now();
        let r = s
            .wait_for(&re("never"), s.end(), Duration::from_millis(30), WIN)
            .unwrap();
        assert_eq!(r, None);
        assert!(started.elapsed() >= Duration::from_millis(30));
    }

    #[test]
    fn wait_for_wakes_up_when_data_arrives_later() {
        let (s, d, _c) = session();
        let from = s.end();
        let feeder = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            d.send(b"+CSQ: 23,99\r\nOK\r\n");
            d
        });
        let m = s
            .wait_for(&re(r"\+CSQ: (\d+)"), from, LONG, WIN)
            .unwrap()
            .unwrap();
        assert_eq!(m.matched, b"+CSQ: 23");
        drop(feeder.join().unwrap());
    }

    #[test]
    fn next_cursor_lets_consecutive_waits_see_later_matches_only() {
        let (s, d, _c) = session();
        d.send(b"A1\r\n");
        let first = s
            .wait_for(&re(r"A\d"), Cursor(0), LONG, WIN)
            .unwrap()
            .unwrap();
        assert_eq!(first.matched, b"A1");
        d.send(b"A2\r\n");
        let second = s
            .wait_for(&re(r"A\d"), first.next, LONG, WIN)
            .unwrap()
            .unwrap();
        assert_eq!(second.matched, b"A2");
    }

    #[test]
    fn send_and_expect_matches_only_the_reply() {
        let (s, d, _c) = session();
        d.send(b"OK stale\r\n");
        assert!(
            s.wait_for(&re("stale"), Cursor(0), LONG, WIN)
                .unwrap()
                .is_some()
        );
        let replier = std::thread::spawn(move || {
            while d.take_written().is_empty() {
                std::thread::sleep(Duration::from_millis(1));
            }
            d.send(b"\r\nOK\r\n");
            d
        });
        let m = s
            .send_and_expect(b"AT\r\n", &re(r"OK\r\n"), LONG, WIN)
            .unwrap()
            .unwrap();
        assert_eq!(m.matched, b"OK\r\n");
        drop(replier.join().unwrap());
    }

    /// Control half that makes the device answer *inside* `write_all`, and
    /// waits until the reader has logged the answer before returning, so the
    /// reply is logged before the command.
    struct InstantReply {
        inner: Box<dyn TransportControl>,
        device: MockDevice,
        reply: &'static [u8],
        logged: Arc<Shared>,
    }

    impl TransportControl for InstantReply {
        fn write_all(&mut self, data: &[u8]) -> Result<(), TransportError> {
            self.inner.write_all(data)?;
            let before = self.logged.lock().log.end();
            self.device.send(self.reply);
            while self.logged.lock().log.end() == before {
                std::thread::sleep(Duration::from_millis(1));
            }
            Ok(())
        }
        fn set_dtr(&mut self, l: bool) -> Result<(), TransportError> {
            self.inner.set_dtr(l)
        }
        fn set_rts(&mut self, l: bool) -> Result<(), TransportError> {
            self.inner.set_rts(l)
        }
        fn line_states(&mut self) -> Result<LineStates, TransportError> {
            self.inner.line_states()
        }
        fn send_break(&mut self, d: Duration) -> Result<(), TransportError> {
            self.inner.send_break(d)
        }
        fn set_baud_rate(&mut self, b: u32) -> Result<(), TransportError> {
            self.inner.set_baud_rate(b)
        }
    }

    /// Puts `control` in place of the session's control half, which is
    /// returned so a wrapper can forward to it.
    fn swap_control(s: &Session, control: Box<dyn TransportControl>) -> Box<dyn TransportControl> {
        std::mem::replace(&mut *lock(&s.control), control)
    }

    #[test]
    fn a_reply_during_the_write_is_logged_after_the_command() {
        // Regression: the TX entry was logged after write_all returned, so a
        // device answering within the write showed its reply before the
        // command (seen in the app with a fast PTY device).
        let (s, d, _c) = session();
        let original = swap_control(&s, mock::connection("unused", T).0.control);
        swap_control(
            &s,
            Box::new(InstantReply {
                inner: original,
                device: d,
                reply: b"OK\r\n",
                logged: Arc::clone(&s.shared),
            }),
        );

        let m = s
            .send_and_expect(b"AT\r\n", &re(r"OK"), Duration::from_millis(200), WIN)
            .unwrap();
        assert!(m.is_some(), "the reply was found");
        let entries = s.read(Cursor(0), usize::MAX).unwrap().entries;
        let directions: Vec<Direction> = entries
            .iter()
            .map(|e| match &e.kind {
                EntryKind::Data { direction, .. } => *direction,
                EntryKind::Marker { .. } => panic!("unexpected marker"),
            })
            .collect();
        assert_eq!(directions, [Direction::Tx, Direction::Rx]);
    }

    /// A control half whose writes fail.
    struct FailingWrite;

    impl TransportControl for FailingWrite {
        fn write_all(&mut self, _: &[u8]) -> Result<(), TransportError> {
            Err(TransportError::Io(std::io::Error::other("cable cut")))
        }
        fn set_dtr(&mut self, _: bool) -> Result<(), TransportError> {
            Ok(())
        }
        fn set_rts(&mut self, _: bool) -> Result<(), TransportError> {
            Ok(())
        }
        fn line_states(&mut self) -> Result<LineStates, TransportError> {
            Ok(LineStates::default())
        }
        fn send_break(&mut self, _: Duration) -> Result<(), TransportError> {
            Ok(())
        }
        fn set_baud_rate(&mut self, _: u32) -> Result<(), TransportError> {
            Ok(())
        }
    }

    #[test]
    fn a_failed_write_is_marked_in_the_log() {
        let (s, _d, _c) = session();
        swap_control(&s, Box::new(FailingWrite));
        let err = s.write(b"AT\r\n").unwrap_err();
        assert!(err.to_string().contains("cable cut"), "{err}");
        let entries = s.read(Cursor(0), usize::MAX).unwrap().entries;
        assert_eq!(entries.len(), 2, "the attempt and its failure");
        assert!(matches!(
            entries[0].kind,
            EntryKind::Data {
                direction: Direction::Tx,
                ..
            }
        ));
        assert!(matches!(
            &entries[1].kind,
            EntryKind::Marker { label } if label == "TX failed: I/O error: cable cut"
        ));
    }

    #[test]
    fn disconnect_wakes_waiters_and_blocks_writes() {
        let (s, d, _c) = session();
        let from = s.end();
        let unplug = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            d.send(b"last");
            d.disconnect();
        });
        let r = s.wait_for(&re("never"), from, LONG, WIN);
        assert!(matches!(
            r,
            Err(SessionError::NotOpen(SessionState::Disconnected))
        ));
        unplug.join().unwrap();
        assert_eq!(s.state(), SessionState::Disconnected);
        assert_eq!(rx_bytes(&s), b"last", "data before unplug is kept");
        assert!(matches!(s.write(b"x"), Err(SessionError::NotOpen(_))));
        assert!(matches!(s.set_dtr(true), Err(SessionError::NotOpen(_))));
        // Markers still work on a dead session.
        s.mark("unplugged");
    }

    #[test]
    fn data_received_before_disconnect_still_matches() {
        let (s, d, _c) = session();
        let from = s.end();
        d.send(b"panic'ed (LoadProhibited)");
        d.disconnect();
        // Give the reader time to see the disconnect.
        while s.state() == SessionState::Open {
            std::thread::sleep(Duration::from_millis(1));
        }
        let m = s.wait_for(&re("LoadProhibited"), from, LONG, WIN).unwrap();
        assert!(m.is_some());
    }

    #[test]
    fn control_calls_pass_through_while_open() {
        let (s, d, _c) = session();
        s.set_dtr(true).unwrap();
        s.set_rts(true).unwrap();
        s.send_break(Duration::from_millis(1)).unwrap();
        s.set_baud_rate(921_600).unwrap();
        let lines = s.line_states().unwrap();
        assert_eq!(lines, LineStates::default());
        assert!(d.dtr() && d.rts());
        assert_eq!(d.baud_rate(), 921_600);
        assert_eq!(d.breaks().len(), 1);
    }

    #[test]
    fn output_signals_are_remembered_and_logged() {
        let (s, d, _c) = session();
        assert_eq!(s.output_signals(), OutputSignals::default());
        s.set_dtr(false).unwrap();
        s.set_rts(true).unwrap();
        assert_eq!(
            s.output_signals(),
            OutputSignals {
                dtr: Some(false),
                rts: Some(true)
            }
        );
        assert!(!d.dtr() && d.rts());
        let labels: Vec<String> = s
            .read(Cursor(0), usize::MAX)
            .unwrap()
            .entries
            .into_iter()
            .map(|e| match e.kind {
                EntryKind::Marker { label } => label,
                other @ EntryKind::Data { .. } => panic!("unexpected {other:?}"),
            })
            .collect();
        assert_eq!(labels, ["DTR=0", "RTS=1"]);
    }

    #[test]
    fn rts_is_refused_under_hardware_flow_control() {
        let (mut conn, dev) = mock::connection("mock0", T);
        conn.info.capabilities.manual_rts = false;
        let s = Session::start(
            SessionId(1),
            conn,
            Arc::new(ManualClock::new(0)),
            SessionOptions::default(),
        )
        .unwrap();
        let before = dev.rts();
        let err = s.set_rts(!before).unwrap_err();
        assert!(err.to_string().contains("hardware flow control"), "{err}");
        assert_eq!(dev.rts(), before, "the driver was not asked");
        assert_eq!(s.output_signals().rts, None);
        assert_eq!(s.end(), Cursor(0), "nothing logged");
        // DTR is unaffected.
        s.set_dtr(false).unwrap();
        assert!(!dev.dtr());
    }

    #[test]
    fn failed_signal_changes_are_not_remembered() {
        let (s, d, _c) = session();
        d.disconnect();
        while s.state() == SessionState::Open {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(s.set_dtr(true).is_err());
        assert_eq!(s.output_signals(), OutputSignals::default());
        assert_eq!(s.end(), Cursor(0), "nothing logged");
    }

    #[test]
    fn close_is_prompt_idempotent_and_keeps_the_log() {
        let (s, d, _c) = session();
        d.send(b"keep me");
        s.wait_for(&re("keep me"), Cursor(0), LONG, WIN).unwrap();
        let started = Instant::now();
        s.close();
        s.close();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(s.state(), SessionState::Closed);
        assert_eq!(rx_bytes(&s), b"keep me");
        assert!(matches!(
            s.write(b"x"),
            Err(SessionError::NotOpen(SessionState::Closed))
        ));
    }

    #[test]
    fn markers_are_logged_and_wake_readers() {
        let (s, _d, clock) = session();
        clock.advance(7);
        let seq = s.mark("reset");
        let r = s.read(Cursor(seq), usize::MAX).unwrap();
        assert_eq!(r.entries[0].timestamp_us, 1_007);
        assert_eq!(
            r.entries[0].kind,
            EntryKind::Marker {
                label: "reset".into()
            }
        );
    }

    #[test]
    fn alias_can_be_set_and_cleared() {
        let (s, _d, _c) = session();
        assert_eq!(s.alias(), None);
        s.set_alias(Some("main board".into()));
        assert_eq!(s.alias().as_deref(), Some("main board"));
        s.set_alias(None);
        assert_eq!(s.alias(), None);
    }

    #[test]
    fn invalid_cursor_is_reported() {
        let (s, _d, _c) = session();
        assert!(matches!(
            s.read(Cursor(99), 1),
            Err(SessionError::Log(LogError::CursorAhead { .. }))
        ));
    }

    #[test]
    fn zero_log_capacity_is_rejected() {
        let (conn, _d) = mock::connection("m", T);
        let opts = SessionOptions {
            log_capacity: 0,
            ..SessionOptions::default()
        };
        let r = Session::start(SessionId(1), conn, Arc::new(ManualClock::new(0)), opts);
        assert!(matches!(r, Err(SessionError::Log(LogError::ZeroCapacity))));
    }

    #[test]
    fn match_window_trims_old_bytes_but_keeps_seq_mapping() {
        let mut w = MatchWindow::new(8);
        w.push(1, b"abcdef");
        w.push(2, b"ghij");
        w.push(3, b"");
        // Only the last 8 bytes remain: "cdefghij".
        assert_eq!(w.bytes, b"cdefghij");
        let m = w.find(&re("fgh")).unwrap();
        assert_eq!(m.seq, 2, "match ends in the second chunk");
        let m = w.find(&re("cde")).unwrap();
        assert_eq!(m.seq, 1);
        assert!(w.find(&re("ab")).is_none(), "trimmed bytes are gone");
        w.push(4, b"0123456789");
        assert_eq!(w.bytes, b"23456789");
        assert_eq!(w.find(&re("2")).unwrap().seq, 4);
    }

    #[test]
    fn empty_match_maps_to_its_entry() {
        let mut w = MatchWindow::new(16);
        w.push(5, b"xy");
        let m = w.find(&re("^")).unwrap();
        assert_eq!((m.seq, m.matched.len()), (5, 0));
    }
}
