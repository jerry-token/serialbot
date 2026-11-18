//! Sending a file: its bytes are written to a session in chunks, with an
//! optional pause after each chunk for devices that cannot keep up (no
//! flow control, small receive buffers).
//!
//! Each session has at most one file job, on its own thread, so the window
//! stays responsive and can show progress or stop it. Markers in the log
//! say which file was sent and how it ended.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::Serialize;
use serialbot_core::Session;

/// Largest file accepted (64 MiB); serial links rarely carry more and the
/// whole file is read into memory.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Largest chunk written at once.
pub const MAX_CHUNK: usize = 64 * 1024;
/// Longest pause after a chunk (one minute).
pub const MAX_DELAY_MS: u64 = 60_000;

/// Progress of a session's file job.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSendStatus {
    /// Whether the job is still sending.
    pub running: bool,
    /// File name (without directories) of the current or last job.
    pub name: Option<String>,
    /// Bytes written so far.
    pub sent: u64,
    /// Size of the file.
    pub total: u64,
    /// Why the job stopped early, if a write failed.
    pub error: Option<String>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Job {
    /// Set to stop; the condvar wakes the thread from a pause.
    stop: Arc<(Mutex<bool>, Condvar)>,
    status: Arc<Mutex<FileSendStatus>>,
    thread: Option<JoinHandle<()>>,
}

impl Job {
    fn stop_and_join(&mut self) {
        *lock(&self.stop.0) = true;
        self.stop.1.notify_all();
        if let Some(thread) = self.thread.take() {
            // A panicked job has nothing left to clean up.
            let _ = thread.join();
        }
    }
}

/// The file jobs of all sessions. A finished job's status stays readable
/// until the next job of that session starts or the session is removed.
#[derive(Default)]
pub struct FileSends {
    jobs: Mutex<HashMap<u32, Job>>,
}

impl std::fmt::Debug for FileSends {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSends")
            .field("jobs", &lock(&self.jobs).len())
            .finish()
    }
}

impl FileSends {
    /// Reads the file at `path` and starts writing it to `session` in
    /// chunks of `chunk` bytes, pausing `delay_ms` after each chunk but the
    /// last. Stops the session's previous job first.
    ///
    /// Fails if the file cannot be read, is empty or larger than
    /// [`MAX_FILE_BYTES`], or `chunk`/`delay_ms` are outside
    /// 1..=[`MAX_CHUNK`] / 0..=[`MAX_DELAY_MS`].
    pub fn start(
        &self,
        id: u32,
        session: Arc<Session>,
        path: &Path,
        chunk: usize,
        delay_ms: u64,
    ) -> Result<FileSendStatus, String> {
        if !(1..=MAX_CHUNK).contains(&chunk) {
            return Err(format!(
                "the chunk size must be between 1 and {MAX_CHUNK} bytes"
            ));
        }
        if delay_ms > MAX_DELAY_MS {
            return Err(format!("the delay must be at most {MAX_DELAY_MS} ms"));
        }
        let size = std::fs::metadata(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?
            .len();
        if size > MAX_FILE_BYTES {
            return Err(format!(
                "the file is too large ({size} bytes; at most {MAX_FILE_BYTES})"
            ));
        }
        let data =
            std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        if data.is_empty() {
            return Err("the file is empty".into());
        }
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        self.stop(id);
        let status = Arc::new(Mutex::new(FileSendStatus {
            running: true,
            name: Some(name.clone()),
            sent: 0,
            total: data.len() as u64,
            error: None,
        }));
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let thread = {
            let stop = Arc::clone(&stop);
            let status = Arc::clone(&status);
            let delay = Duration::from_millis(delay_ms);
            std::thread::spawn(move || run(&session, &name, &data, chunk, delay, &stop, &status))
        };
        let started = lock(&status).clone();
        lock(&self.jobs).insert(
            id,
            Job {
                stop,
                status,
                thread: Some(thread),
            },
        );
        Ok(started)
    }

    /// Stops the session's job, if one is running, and waits for it to end.
    /// Its final status stays readable.
    pub fn stop(&self, id: u32) {
        let job = lock(&self.jobs).remove(&id);
        if let Some(mut job) = job {
            job.stop_and_join();
            lock(&self.jobs).entry(id).or_insert(job);
        }
    }

    /// Progress of the session's current or last job; a session without one
    /// reports "not running, nothing sent".
    pub fn status(&self, id: u32) -> FileSendStatus {
        lock(&self.jobs)
            .get(&id)
            .map(|job| lock(&job.status).clone())
            .unwrap_or_default()
    }

    /// Stops the session's job and forgets it (the session was closed).
    pub fn remove(&self, id: u32) {
        let job = lock(&self.jobs).remove(&id);
        if let Some(mut job) = job {
            job.stop_and_join();
        }
    }

    /// Stops every job (on app exit).
    pub fn stop_all(&self) {
        let jobs: Vec<Job> = lock(&self.jobs).drain().map(|(_, job)| job).collect();
        for mut job in jobs {
            job.stop_and_join();
        }
    }
}

fn run(
    session: &Session,
    name: &str,
    data: &[u8],
    chunk: usize,
    delay: Duration,
    stop: &(Mutex<bool>, Condvar),
    status: &Mutex<FileSendStatus>,
) {
    let total = data.len();
    session.mark(&format!("send file {name} ({total} bytes)"));
    let mut sent = 0;
    let mut end = None;
    for part in data.chunks(chunk) {
        if *lock(&stop.0) {
            end = Some(format!("file send stopped at {sent}/{total} bytes"));
            break;
        }
        if let Err(e) = session.write(part) {
            let e = e.to_string();
            end = Some(format!("file send failed at {sent}/{total} bytes: {e}"));
            lock(status).error = Some(e);
            break;
        }
        sent += part.len();
        lock(status).sent = sent as u64;
        if sent < total && !delay.is_zero() {
            let stopped = lock(&stop.0);
            // Woken early by stop; the loop then reports it.
            drop(
                stop.1
                    .wait_timeout_while(stopped, delay, |s| !*s)
                    .unwrap_or_else(PoisonError::into_inner),
            );
        }
    }
    session.mark(end.as_deref().unwrap_or("file sent"));
    lock(status).running = false;
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use serialbot_core::transport::mock::{self, MockDevice};
    use serialbot_core::{Cursor, Direction, EntryKind, SessionId, SessionOptions, SystemClock};

    use super::*;
    use crate::settings::tests::TempDir;

    fn session() -> (Arc<Session>, MockDevice) {
        let (connection, device) = mock::connection("mock0", Duration::from_millis(5));
        let session = Session::start(
            SessionId(1),
            connection,
            Arc::new(SystemClock),
            SessionOptions::default(),
        )
        .unwrap();
        (Arc::new(session), device)
    }

    fn wait_until(check: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !check() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn file(dir: &TempDir, name: &str, data: &[u8]) -> std::path::PathBuf {
        let path = dir.0.join(name);
        std::fs::write(&path, data).unwrap();
        path
    }

    fn markers(s: &Session) -> Vec<String> {
        s.read(Cursor(0), usize::MAX)
            .unwrap()
            .entries
            .into_iter()
            .filter_map(|e| match e.kind {
                EntryKind::Marker { label } => Some(label),
                EntryKind::Data { .. } => None,
            })
            .collect()
    }

    #[test]
    fn sends_the_file_in_chunks_and_marks_the_log() {
        let dir = TempDir::new("filesend-chunks");
        let path = file(&dir, "fw.bin", b"0123456789");
        let (s, d) = session();
        let jobs = FileSends::default();
        let started = jobs.start(1, Arc::clone(&s), &path, 4, 1).unwrap();
        assert!(started.running);
        assert_eq!(started.name.as_deref(), Some("fw.bin"));
        assert_eq!(started.total, 10);
        wait_until(|| !jobs.status(1).running);
        assert_eq!(jobs.status(1).sent, 10);
        assert_eq!(d.take_written(), b"0123456789");
        let tx: Vec<usize> = s
            .read(Cursor(0), usize::MAX)
            .unwrap()
            .entries
            .iter()
            .filter_map(|e| match &e.kind {
                EntryKind::Data {
                    direction: Direction::Tx,
                    bytes,
                } => Some(bytes.len()),
                _ => None,
            })
            .collect();
        assert_eq!(tx, [4, 4, 2]);
        assert_eq!(markers(&s), ["send file fw.bin (10 bytes)", "file sent"]);
    }

    #[test]
    fn stops_promptly_during_a_pause() {
        let dir = TempDir::new("filesend-stop");
        let path = file(&dir, "big.txt", b"abcdef");
        let (s, d) = session();
        let jobs = FileSends::default();
        jobs.start(1, Arc::clone(&s), &path, 2, MAX_DELAY_MS)
            .unwrap();
        wait_until(|| jobs.status(1).sent == 2);
        let started = Instant::now();
        jobs.stop(1);
        assert!(started.elapsed() < Duration::from_secs(5));
        let status = jobs.status(1);
        assert!(!status.running);
        assert_eq!(status.sent, 2, "final status stays readable");
        assert_eq!(d.take_written(), b"ab");
        assert_eq!(markers(&s)[1], "file send stopped at 2/6 bytes");

        // A new job replaces the finished one.
        jobs.start(1, Arc::clone(&s), &path, 64, 0).unwrap();
        wait_until(|| !jobs.status(1).running);
        assert_eq!(jobs.status(1).sent, 6);
        jobs.remove(1);
        assert_eq!(jobs.status(1), FileSendStatus::default());
    }

    #[test]
    fn stops_with_the_error_when_a_write_fails() {
        let dir = TempDir::new("filesend-fail");
        let path = file(&dir, "x.bin", b"xyz");
        let (s, _d) = session();
        s.close();
        let jobs = FileSends::default();
        jobs.start(1, Arc::clone(&s), &path, 1, 0).unwrap();
        wait_until(|| !jobs.status(1).running);
        let status = jobs.status(1);
        assert_eq!(status.sent, 0);
        assert!(status.error.unwrap().contains("Closed"));
    }

    #[test]
    fn rejects_bad_requests() {
        let dir = TempDir::new("filesend-bad");
        let (s, _d) = session();
        let jobs = FileSends::default();
        let ok = file(&dir, "ok.bin", b"1");
        let empty = file(&dir, "empty.bin", b"");
        let err = |path: &Path, chunk, delay| {
            jobs.start(1, Arc::clone(&s), path, chunk, delay)
                .unwrap_err()
        };
        assert!(err(&ok, 0, 0).contains("chunk size"));
        assert!(err(&ok, MAX_CHUNK + 1, 0).contains("chunk size"));
        assert!(err(&ok, 1, MAX_DELAY_MS + 1).contains("delay"));
        assert!(err(&empty, 1, 0).contains("empty"));
        assert!(err(&dir.0.join("missing.bin"), 1, 0).contains("cannot read"));
        assert!(err(&dir.0, 1, 0).contains("cannot read"), "a directory");
        assert_eq!(jobs.status(1), FileSendStatus::default());
    }

    #[test]
    fn stop_all_ends_every_job() {
        let dir = TempDir::new("filesend-all");
        let path = file(&dir, "a.bin", b"aaaa");
        let jobs = FileSends::default();
        let (s1, _d1) = session();
        let (s2, _d2) = session();
        jobs.start(1, s1, &path, 1, MAX_DELAY_MS).unwrap();
        jobs.start(2, s2, &path, 1, MAX_DELAY_MS).unwrap();
        assert!(format!("{jobs:?}").contains("jobs: 2"));
        jobs.stop_all();
        assert!(!jobs.status(1).running);
        assert_eq!(jobs.status(2), FileSendStatus::default());
    }
}
