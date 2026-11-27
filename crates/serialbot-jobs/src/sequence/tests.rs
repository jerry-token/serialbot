// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
use std::sync::atomic::AtomicUsize;

use serialbot_core::log::EntryKind;
use serialbot_core::transport::mock::{self, MockDevice};
use serialbot_core::{SessionId, SessionOptions, SystemClock};

use super::*;
use crate::script::{Ending, parse_text};

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

/// Answers each `ping` with `OK`, except the pings numbered in `silent`
/// (from 1). Returns how many pings it saw.
fn responder(device: MockDevice, silent: &'static [usize]) -> Arc<AtomicUsize> {
    let pings = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&pings);
    std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            let got = device.take_written();
            for _ in 0..got.windows(4).filter(|w| w == b"ping").count() {
                let n = seen.fetch_add(1, Ordering::Relaxed) + 1;
                if !silent.contains(&n) {
                    device.send(b"OK\r\n");
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    pings
}

fn script() -> Vec<Item> {
    parse_text("timeout 300ms\nsend ping\nexpect OK", Ending::Crlf, "utf-8").unwrap()
}

fn markers(session: &Session) -> Vec<String> {
    session
        .read(serialbot_core::log::Cursor::default(), 1 << 20)
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
fn counts_passed_runs_and_their_times() {
    let (s, device) = session();
    let pings = responder(device, &[]);
    let status = Mutex::new(SequenceStatus::default());
    let mut steps = Vec::new();
    let options = Options {
        runs: Some(3),
        ..Options::default()
    };
    run_loops(
        &s,
        "ping",
        &script(),
        &options,
        &AtomicBool::new(false),
        &status,
        &mut |run, p| steps.push((run, p.line, p.stop.is_none())),
    );
    let st = lock(&status).clone();
    assert!(!st.running && !st.stopped && st.error.is_none(), "{st:?}");
    assert_eq!((st.name.as_str(), st.runs), ("ping", Some(3)));
    assert_eq!((st.done, st.passed, st.failed), (3, 3, 0));
    assert!(st.failures.is_empty());
    let (fastest, slowest) = (st.fastest_ms.unwrap(), st.slowest_ms.unwrap());
    assert!(fastest <= slowest && slowest <= st.passed_ms, "{st:?}");
    assert!(st.passed_ms <= st.elapsed_ms, "{st:?}");
    assert_eq!(st.step, None);
    assert_eq!(pings.load(Ordering::Relaxed), 3);
    assert_eq!(
        steps,
        [
            (1, 2, true),
            (1, 3, true),
            (2, 2, true),
            (2, 3, true),
            (3, 2, true),
            (3, 3, true)
        ]
    );
    assert_eq!(markers(&s), ["run 1/3", "run 2/3", "run 3/3"]);
}

#[test]
fn one_run_puts_no_marker() {
    let (s, device) = session();
    responder(device, &[]);
    let status = Mutex::new(SequenceStatus::default());
    run_loops(
        &s,
        "once",
        &script(),
        &Options::default(),
        &AtomicBool::new(false),
        &status,
        &mut |_, _| {},
    );
    assert_eq!(lock(&status).passed, 1);
    assert!(markers(&s).is_empty());
}

#[test]
fn failed_runs_are_counted_and_the_sequence_goes_on() {
    let (s, device) = session();
    responder(device, &[2]);
    let status = Mutex::new(SequenceStatus::default());
    let options = Options {
        runs: Some(4),
        stop_on_fail: false,
        gap: Duration::ZERO,
    };
    run_loops(
        &s,
        "ping",
        &script(),
        &options,
        &AtomicBool::new(false),
        &status,
        &mut |_, _| {},
    );
    let st = lock(&status).clone();
    assert_eq!((st.done, st.passed, st.failed), (4, 3, 1), "{st:?}");
    assert_eq!(
        st.failures,
        [Failure {
            run: 2,
            line: 3,
            source: "expect OK".into(),
            reason: Reason::Timeout,
        }]
    );
    assert!(st.error.is_none() && !st.stopped);
    assert_eq!(
        serde_json::to_value(&st.failures[0]).unwrap(),
        serde_json::json!({
            "run": 2, "line": 3, "source": "expect OK", "reason": { "kind": "timeout" }
        })
    );
}

#[test]
fn stop_on_fail_ends_at_the_first_failed_run() {
    let (s, device) = session();
    responder(device, &[2]);
    let status = Mutex::new(SequenceStatus::default());
    let options = Options {
        runs: Some(5),
        stop_on_fail: true,
        gap: Duration::ZERO,
    };
    run_loops(
        &s,
        "ping",
        &script(),
        &options,
        &AtomicBool::new(false),
        &status,
        &mut |_, _| {},
    );
    let st = lock(&status).clone();
    assert_eq!((st.done, st.passed, st.failed), (2, 1, 1), "{st:?}");
    assert!(!st.stopped);
}

#[test]
fn the_device_going_away_ends_the_sequence() {
    let (s, device) = session();
    device.disconnect();
    std::thread::sleep(Duration::from_millis(50));
    let status = Mutex::new(SequenceStatus::default());
    let options = Options {
        runs: None,
        stop_on_fail: false,
        gap: Duration::ZERO,
    };
    run_loops(
        &s,
        "ping",
        &script(),
        &options,
        &AtomicBool::new(false),
        &status,
        &mut |_, _| {},
    );
    let st = lock(&status).clone();
    assert_eq!((st.done, st.failed), (1, 1), "{st:?}");
    assert!(st.error.is_some());
    assert!(matches!(st.failures[0].reason, Reason::Error(_)));
    assert_eq!(st.failures[0].line, 2);
}

#[test]
fn only_the_first_failures_are_kept() {
    let (s, _device) = session();
    let status = Mutex::new(SequenceStatus::default());
    let items = parse_text("timeout 0\nexpect never", Ending::None, "utf-8").unwrap();
    let runs = u32::try_from(MAX_FAILURES).unwrap() + 3;
    let options = Options {
        runs: Some(runs),
        stop_on_fail: false,
        gap: Duration::ZERO,
    };
    run_loops(
        &s,
        "never",
        &items,
        &options,
        &AtomicBool::new(false),
        &status,
        &mut |_, _| {},
    );
    let st = lock(&status).clone();
    assert_eq!(st.failed, runs);
    assert_eq!(st.failures.len(), MAX_FAILURES);
    assert_eq!(st.fastest_ms, None);
}

#[test]
fn background_sequences_run_stop_and_keep_their_status() {
    let (s, device) = session();
    responder(device, &[]);
    let sequences = Sequences::default();
    assert_eq!(sequences.status(1), SequenceStatus::default());
    assert!(format!("{sequences:?}").contains("Sequences"));

    let options = Options {
        runs: None,
        stop_on_fail: true,
        gap: Duration::from_millis(20),
    };
    sequences
        .start(1, Arc::clone(&s), "soak".into(), script(), options)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while sequences.status(1).passed < 3 {
        assert!(Instant::now() < deadline, "{:?}", sequences.status(1));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(sequences.status(1).running);
    sequences.stop(1);
    let st = sequences.status(1);
    assert!(!st.running && st.stopped, "{st:?}");
    assert_eq!(st.name, "soak");
    assert_eq!(st.runs, None);
    assert!(st.passed >= 3 && st.failed == 0, "{st:?}");
    // Stopping again changes nothing.
    sequences.stop(1);
    assert_eq!(sequences.status(1), st);

    // A new one replaces it.
    sequences
        .start(
            1,
            Arc::clone(&s),
            "again".into(),
            script(),
            Options::default(),
        )
        .unwrap();
    while sequences.status(1).running {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(sequences.status(1).passed, 1);
    sequences.remove(1);
    assert_eq!(sequences.status(1), SequenceStatus::default());
    sequences.remove(1);

    sequences
        .start(2, Arc::clone(&s), "x".into(), script(), Options::default())
        .unwrap();
    sequences.stop_all();
    assert!(!sequences.status(2).running);
}

#[test]
fn a_stop_during_the_pause_between_runs_ends_it_quickly() {
    let (s, device) = session();
    responder(device, &[]);
    let sequences = Sequences::default();
    let options = Options {
        runs: Some(2),
        stop_on_fail: true,
        gap: Duration::from_secs(30),
    };
    sequences
        .start(1, s, "slow".into(), script(), options)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while sequences.status(1).passed < 1 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    let started = Instant::now();
    sequences.stop(1);
    assert!(started.elapsed() < Duration::from_secs(2));
    let st = sequences.status(1);
    assert!(st.stopped && st.passed == 1, "{st:?}");
}

#[test]
fn bad_sequences_are_refused() {
    let (s, _device) = session();
    let sequences = Sequences::default();
    let start = |items, options| sequences.start(1, Arc::clone(&s), "x".into(), items, options);
    assert!(start(Vec::new(), Options::default()).is_err());
    let zero = Options {
        runs: Some(0),
        ..Options::default()
    };
    assert!(start(script(), zero).is_err());
    let long = Options {
        gap: MAX_GAP + Duration::from_secs(1),
        ..Options::default()
    };
    assert!(start(script(), long).is_err());
    assert_eq!(sequences.status(1), SequenceStatus::default());
}
