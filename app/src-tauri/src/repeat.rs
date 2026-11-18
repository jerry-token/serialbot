//! Timed sending: write the same bytes to a session every N milliseconds,
//! a fixed number of times or until stopped.
//!
//! Each session has at most one repeat job, run on its own thread so it
//! keeps time even while the window is hidden (webviews throttle timers in
//! background windows). Sends are scheduled from the previous due time, so
//! the rate does not drift; if a write takes longer than the interval the
//! next send follows immediately instead of bursting to catch up.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;
use serialx_core::Session;

/// Shortest accepted interval.
pub const MIN_INTERVAL_MS: u64 = 1;
/// Longest accepted interval (24 hours).
pub const MAX_INTERVAL_MS: u64 = 24 * 60 * 60 * 1000;

/// Progress of a session's repeat job.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepeatStatus {
    /// Whether the job is still sending.
    pub running: bool,
    /// How many times the data was sent.
    pub sent: u32,
    /// Why the job stopped early, if a write failed.
    pub error: Option<String>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

struct Job {
    /// Set to stop; the condvar wakes the thread from its wait.
    stop: Arc<(Mutex<bool>, Condvar)>,
    status: Arc<Mutex<RepeatStatus>>,
    thread: Option<JoinHandle<()>>,
}

impl Job {
    fn stop_and_join(mut self) {
        *lock(&self.stop.0) = true;
        self.stop.1.notify_all();
        if let Some(thread) = self.thread.take() {
            // A panicked job has nothing left to clean up.
            let _ = thread.join();
        }
    }
}

/// The repeat jobs of all sessions.
#[derive(Default)]
pub struct Repeaters {
    jobs: Mutex<HashMap<u32, Job>>,
}

impl std::fmt::Debug for Repeaters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Repeaters")
            .field("jobs", &lock(&self.jobs).len())
            .finish()
    }
}

impl Repeaters {
    /// Starts sending `data` to `session` every `interval_ms`, `count` times
    /// (`None`: until stopped). Replaces the session's previous job.
    ///
    /// Fails for empty data, a zero count, or an interval outside
    /// [`MIN_INTERVAL_MS`]..=[`MAX_INTERVAL_MS`].
    pub fn start(
        &self,
        id: u32,
        session: Arc<Session>,
        data: Vec<u8>,
        interval_ms: u64,
        count: Option<u32>,
    ) -> Result<(), String> {
        if data.is_empty() {
            return Err("nothing to send".into());
        }
        if !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&interval_ms) {
            return Err(format!(
                "the interval must be between {MIN_INTERVAL_MS} ms and {MAX_INTERVAL_MS} ms"
            ));
        }
        if count == Some(0) {
            return Err("the count must be at least 1".into());
        }
        self.stop(id);
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let status = Arc::new(Mutex::new(RepeatStatus {
            running: true,
            ..RepeatStatus::default()
        }));
        let thread = {
            let stop = Arc::clone(&stop);
            let status = Arc::clone(&status);
            let interval = Duration::from_millis(interval_ms);
            std::thread::spawn(move || run(&session, &data, interval, count, &stop, &status))
        };
        lock(&self.jobs).insert(
            id,
            Job {
                stop,
                status,
                thread: Some(thread),
            },
        );
        Ok(())
    }

    /// Stops the session's job, if any, and waits for it to end.
    pub fn stop(&self, id: u32) {
        let job = lock(&self.jobs).remove(&id);
        if let Some(job) = job {
            job.stop_and_join();
        }
    }

    /// Progress of the session's job; a session without one reports
    /// "not running, nothing sent".
    pub fn status(&self, id: u32) -> RepeatStatus {
        lock(&self.jobs)
            .get(&id)
            .map(|job| lock(&job.status).clone())
            .unwrap_or_default()
    }

    /// Stops every job (on app exit).
    pub fn stop_all(&self) {
        let jobs: Vec<Job> = lock(&self.jobs).drain().map(|(_, job)| job).collect();
        for job in jobs {
            job.stop_and_join();
        }
    }
}

fn run(
    session: &Session,
    data: &[u8],
    interval: Duration,
    count: Option<u32>,
    stop: &(Mutex<bool>, Condvar),
    status: &Mutex<RepeatStatus>,
) {
    let mut due = Instant::now();
    loop {
        if *lock(&stop.0) {
            break;
        }
        if let Err(e) = session.write(data) {
            lock(status).error = Some(e.to_string());
            break;
        }
        let sent = {
            let mut s = lock(status);
            s.sent += 1;
            s.sent
        };
        if count.is_some_and(|c| sent >= c) {
            break;
        }
        // Next due time; never schedule in the past (no catch-up bursts).
        due = (due + interval).max(Instant::now());
        let mut stopped = lock(&stop.0);
        while !*stopped {
            let now = Instant::now();
            if now >= due {
                break;
            }
            stopped = stop
                .1
                .wait_timeout(stopped, due - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
    lock(status).running = false;
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serialx_core::transport::mock::{self, MockDevice};
    use serialx_core::{SessionId, SessionOptions, SystemClock};

    use super::*;

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

    #[test]
    fn sends_a_fixed_number_of_times() {
        let (s, device) = session();
        let r = Repeaters::default();
        assert_eq!(r.status(1), RepeatStatus::default());
        let started = Instant::now();
        r.start(1, Arc::clone(&s), b"ping\n".to_vec(), 20, Some(3))
            .unwrap();
        wait_until(|| !r.status(1).running);
        assert!(
            started.elapsed() >= Duration::from_millis(40),
            "three sends are two intervals apart"
        );
        assert_eq!(
            r.status(1),
            RepeatStatus {
                running: false,
                sent: 3,
                error: None
            }
        );
        assert_eq!(device.take_written(), b"ping\nping\nping\n");
    }

    #[test]
    fn stops_promptly_and_can_restart() {
        let (s, device) = session();
        let r = Repeaters::default();
        r.start(1, Arc::clone(&s), b"a".to_vec(), 60_000, None)
            .unwrap();
        wait_until(|| r.status(1).sent == 1);
        let stopping = Instant::now();
        r.stop(1);
        assert!(
            stopping.elapsed() < Duration::from_secs(1),
            "not waiting out the interval"
        );
        assert_eq!(r.status(1), RepeatStatus::default(), "the job is gone");
        assert_eq!(device.take_written(), b"a");

        // Starting again replaces any running job.
        r.start(1, Arc::clone(&s), b"b".to_vec(), 60_000, None)
            .unwrap();
        r.start(1, Arc::clone(&s), b"c".to_vec(), 5, Some(2))
            .unwrap();
        wait_until(|| !r.status(1).running);
        assert_eq!(r.status(1).sent, 2);
        let written = device.take_written();
        assert!(written.ends_with(b"cc"), "{written:?}");
        r.stop(9); // No job: nothing happens.
    }

    #[test]
    fn stops_with_the_error_when_a_write_fails() {
        let (s, device) = session();
        device.disconnect();
        wait_until(|| s.state() != serialx_core::SessionState::Open);
        let r = Repeaters::default();
        r.start(1, s, b"x".to_vec(), 5, None).unwrap();
        wait_until(|| !r.status(1).running);
        let status = r.status(1);
        assert_eq!(status.sent, 0);
        assert!(status.error.unwrap().contains("Disconnected"));
    }

    #[test]
    fn rejects_bad_requests() {
        let (s, _device) = session();
        let r = Repeaters::default();
        assert_eq!(
            r.start(1, Arc::clone(&s), Vec::new(), 10, None)
                .unwrap_err(),
            "nothing to send"
        );
        assert!(
            r.start(1, Arc::clone(&s), b"x".to_vec(), 0, None)
                .unwrap_err()
                .contains("interval")
        );
        assert!(
            r.start(1, Arc::clone(&s), b"x".to_vec(), MAX_INTERVAL_MS + 1, None)
                .is_err()
        );
        assert!(
            r.start(1, s, b"x".to_vec(), 10, Some(0))
                .unwrap_err()
                .contains("count")
        );
        assert!(format!("{r:?}").contains("Repeaters"));
    }

    #[test]
    fn stop_all_ends_every_job() {
        let (s, _device) = session();
        let r = Repeaters::default();
        r.start(1, Arc::clone(&s), b"a".to_vec(), 60_000, None)
            .unwrap();
        r.start(2, s, b"b".to_vec(), 60_000, None).unwrap();
        wait_until(|| r.status(1).sent == 1 && r.status(2).sent == 1);
        r.stop_all();
        assert!(!r.status(1).running && !r.status(2).running);
    }
}
