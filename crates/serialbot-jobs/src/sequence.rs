// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Test sequences: a [`script`](mod@crate::script) run over and over, counting
//! the runs that passed and failed, for soak tests and production checks.
//!
//! A run fails when one of its steps fails; the next run starts from the
//! first step again. An `expect` that times out is a failed run (the
//! sequence goes on unless [`Options::stop_on_fail`]); the device going away
//! ends the sequence. Each run after the first is announced with a marker
//! in the log (`run 3/10`), so the log shows where each run starts.
//!
//! [`run_loops`] does the work on the calling thread (the command-line
//! tool); [`Sequences`] runs it in the background, one per session, for the
//! app.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Serialize;
use serialbot_core::Session;

use crate::script::{self, Item, Progress, Stop};

/// How many failures the status keeps (the first ones); the count covers
/// all of them.
pub const MAX_FAILURES: usize = 50;

/// Longest accepted pause between runs (one hour).
pub const MAX_GAP: Duration = Duration::from_secs(60 * 60);

/// How a sequence repeats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// How many runs; `None`: until stopped.
    pub runs: Option<u32>,
    /// End the sequence at the first failed run.
    pub stop_on_fail: bool,
    /// The pause between runs.
    pub gap: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            runs: Some(1),
            stop_on_fail: true,
            gap: Duration::ZERO,
        }
    }
}

/// Why a step failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "message")]
pub enum Reason {
    /// An `expect` saw no match in time.
    Timeout,
    /// A step failed (the device went away, a write failed).
    Error(String),
}

/// A failed run: which run, and the step that failed in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Failure {
    /// The run, from 1.
    pub run: u32,
    /// The step's line in the script (its command number for a command
    /// group).
    pub line: usize,
    /// The step as written.
    pub source: String,
    /// Why it failed.
    pub reason: Reason,
}

/// Where a sequence is, and how it went so far.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceStatus {
    /// Whether it is still running.
    pub running: bool,
    /// What runs (the script's or group's name).
    pub name: String,
    /// How many runs were asked for; `None`: until stopped.
    pub runs: Option<u32>,
    /// Runs finished, passed or failed.
    pub done: u32,
    /// Runs whose every step succeeded.
    pub passed: u32,
    /// Runs with a failed step.
    pub failed: u32,
    /// The step running now, as written.
    pub step: Option<String>,
    /// The first [`MAX_FAILURES`] failures.
    pub failures: Vec<Failure>,
    /// The shortest passed run, in milliseconds.
    pub fastest_ms: Option<u64>,
    /// The longest passed run, in milliseconds.
    pub slowest_ms: Option<u64>,
    /// All passed runs together, in milliseconds (for the average).
    pub passed_ms: u64,
    /// Time since the sequence started, in milliseconds.
    pub elapsed_ms: u64,
    /// Whether it was stopped before its last run.
    pub stopped: bool,
    /// Why the sequence ended early, when the device went away.
    pub error: Option<String>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

/// Runs `items` on `session` as `options` says, on this thread, keeping
/// `status` up to date (it is reset first). `report` gets the run number
/// and each step's outcome. Returns when the runs are done, `stop` is set,
/// a run failed with [`Options::stop_on_fail`], or the device went away.
pub fn run_loops(
    session: &Session,
    name: &str,
    items: &[Item],
    options: &Options,
    stop: &AtomicBool,
    status: &Mutex<SequenceStatus>,
    report: &mut dyn FnMut(u32, &Progress),
) {
    let started = Instant::now();
    *lock(status) = SequenceStatus {
        running: true,
        name: name.to_owned(),
        runs: options.runs,
        ..SequenceStatus::default()
    };
    let mut run = 0u32;
    loop {
        if options.runs.is_some_and(|n| run >= n) {
            break;
        }
        if stop.load(Ordering::Relaxed) {
            lock(status).stopped = true;
            break;
        }
        run += 1;
        if options.runs != Some(1) {
            let of = options.runs.map(|n| format!("/{n}")).unwrap_or_default();
            session.mark(&format!("run {run}{of}"));
        }
        let run_started = Instant::now();
        let mut last = (0, String::new());
        let result = script::run(session, items, stop, &mut |p| {
            last = (p.line, p.source.clone());
            {
                let mut s = lock(status);
                s.step = Some(p.source.clone());
                s.elapsed_ms = millis(started.elapsed());
            }
            report(run, p);
        });
        let failed_step = |reason| {
            let (line, source) = last.clone();
            let mut s = lock(status);
            s.done += 1;
            s.failed += 1;
            if s.failures.len() < MAX_FAILURES {
                s.failures.push(Failure {
                    run,
                    line,
                    source,
                    reason,
                });
            }
        };
        match result {
            Ok(()) => {
                let took = millis(run_started.elapsed());
                let mut s = lock(status);
                s.done += 1;
                s.passed += 1;
                s.passed_ms = s.passed_ms.saturating_add(took);
                s.fastest_ms = Some(s.fastest_ms.map_or(took, |f| f.min(took)));
                s.slowest_ms = Some(s.slowest_ms.map_or(took, |f| f.max(took)));
            }
            Err(Stop::Interrupted) => {
                lock(status).stopped = true;
                break;
            }
            Err(Stop::Timeout) => {
                failed_step(Reason::Timeout);
                if options.stop_on_fail {
                    break;
                }
            }
            Err(Stop::Failed(why)) => {
                failed_step(Reason::Error(why.clone()));
                lock(status).error = Some(why);
                break;
            }
        }
        if options.runs.is_some_and(|n| run >= n) {
            break;
        }
        if !pause(options.gap, stop) {
            lock(status).stopped = true;
            break;
        }
    }
    let mut s = lock(status);
    s.running = false;
    s.step = None;
    s.elapsed_ms = millis(started.elapsed());
}

/// Waits `length`; false if `stop` was set meanwhile.
fn pause(length: Duration, stop: &AtomicBool) -> bool {
    let deadline = Instant::now() + length;
    loop {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return true;
        }
        std::thread::sleep(left.min(Duration::from_millis(50)));
    }
}

struct Job {
    stop: Arc<AtomicBool>,
    status: Arc<Mutex<SequenceStatus>>,
    thread: Option<JoinHandle<()>>,
}

impl Job {
    fn stop_and_join(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            // A panicked job has nothing left to clean up.
            let _ = thread.join();
        }
    }
}

/// The test sequences of all sessions, at most one per session. A finished
/// sequence keeps its status until the next one starts or it is cleared.
#[derive(Default)]
pub struct Sequences {
    jobs: Mutex<HashMap<u32, Job>>,
}

impl std::fmt::Debug for Sequences {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sequences")
            .field("jobs", &lock(&self.jobs).len())
            .finish()
    }
}

impl Sequences {
    /// Starts running `items` on `session` in the background, replacing
    /// the session's previous sequence. Fails for an empty script, zero
    /// runs, or a gap over [`MAX_GAP`].
    pub fn start(
        &self,
        id: u32,
        session: Arc<Session>,
        name: String,
        items: Vec<Item>,
        options: Options,
    ) -> Result<(), String> {
        if items.is_empty() {
            return Err("the sequence has no steps".into());
        }
        if options.runs == Some(0) {
            return Err("the number of runs must be at least 1".into());
        }
        if options.gap > MAX_GAP {
            return Err("the pause between runs must be at most an hour".into());
        }
        self.stop(id);
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(SequenceStatus {
            running: true,
            name: name.clone(),
            runs: options.runs,
            ..SequenceStatus::default()
        }));
        let thread = {
            let stop = Arc::clone(&stop);
            let status = Arc::clone(&status);
            std::thread::spawn(move || {
                run_loops(
                    &session,
                    &name,
                    &items,
                    &options,
                    &stop,
                    &status,
                    &mut |_, _| {},
                );
            })
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

    /// Stops the session's sequence, if one runs, and waits for it to end;
    /// its status stays readable.
    pub fn stop(&self, id: u32) {
        let job = lock(&self.jobs).remove(&id);
        if let Some(job) = job {
            let status = Arc::clone(&job.status);
            job.stop_and_join();
            lock(&self.jobs).insert(
                id,
                Job {
                    stop: Arc::new(AtomicBool::new(true)),
                    status,
                    thread: None,
                },
            );
        }
    }

    /// Stops the session's sequence and forgets it (the session closed).
    pub fn remove(&self, id: u32) {
        let job = lock(&self.jobs).remove(&id);
        if let Some(job) = job {
            job.stop_and_join();
        }
    }

    /// The session's sequence; a session without one reports the default
    /// (not running, no runs).
    pub fn status(&self, id: u32) -> SequenceStatus {
        lock(&self.jobs)
            .get(&id)
            .map(|job| lock(&job.status).clone())
            .unwrap_or_default()
    }

    /// Stops every sequence (on app exit).
    pub fn stop_all(&self) {
        let jobs: Vec<Job> = lock(&self.jobs).drain().map(|(_, job)| job).collect();
        for job in jobs {
            job.stop_and_join();
        }
    }
}

#[cfg(test)]
mod tests;
