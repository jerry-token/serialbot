// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Property-based tests for the invariants of `LogBuffer`.

#![allow(clippy::unwrap_used)]

use bytes::Bytes;
use proptest::prelude::*;
use serialbot_core::{Cursor, Direction, ENTRY_OVERHEAD, EntryKind, LogBuffer};

#[derive(Debug, Clone)]
enum Op {
    Push(Vec<u8>),
    Marker(String),
    /// Incremental read with the given payload limit.
    Read(usize),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => prop::collection::vec(any::<u8>(), 0..64).prop_map(Op::Push),
        1 => "[a-z]{0,8}".prop_map(Op::Marker),
        2 => (0usize..128).prop_map(Op::Read),
    ]
}

proptest! {
    /// Usage never exceeds capacity unless the buffer holds a single entry,
    /// and sequence numbers stay contiguous.
    #[test]
    fn budget_and_contiguity(cap in 1usize..512, ops in prop::collection::vec(op(), 0..200)) {
        let mut log = LogBuffer::new(cap).unwrap();
        for (ts, op) in (0u64..).zip(ops) {
            match op {
                Op::Push(b) => { log.push_data(ts, Direction::Rx, Bytes::from(b)); }
                Op::Marker(m) => { log.push_marker(ts, m); }
                Op::Read(_) => {}
            }
            prop_assert!(log.used() <= log.capacity() || log.len() == 1);
            prop_assert_eq!(log.end().0 - log.start().0, log.len() as u64);
        }
    }

    /// A reader that keeps up sees every pushed entry exactly once, in order,
    /// and the reported `missed` count plus what it saw equals what was pushed.
    #[test]
    fn incremental_reader_sees_everything_once(
        cap in 1usize..512,
        ops in prop::collection::vec(op(), 0..200),
    ) {
        let mut log = LogBuffer::new(cap).unwrap();
        let mut cursor = Cursor::default();
        let mut seen: Vec<u64> = Vec::new();
        let mut missed = 0u64;
        let mut pushed = 0u64;

        let drain = |log: &LogBuffer, cursor: &mut Cursor, limit: usize,
                         seen: &mut Vec<u64>, missed: &mut u64| {
            let r = log.read(*cursor, limit).unwrap();
            *missed += r.missed;
            seen.extend(r.entries.iter().map(|e| e.seq));
            *cursor = r.next;
        };

        for (ts, op) in (0u64..).zip(ops) {
            match op {
                Op::Push(b) => { log.push_data(ts, Direction::Rx, Bytes::from(b)); pushed += 1; }
                Op::Marker(m) => { log.push_marker(ts, m); pushed += 1; }
                Op::Read(limit) => drain(&log, &mut cursor, limit, &mut seen, &mut missed),
            }
        }
        // Drain whatever is left.
        while cursor < log.end() {
            drain(&log, &mut cursor, usize::MAX, &mut seen, &mut missed);
        }

        prop_assert!(seen.windows(2).all(|w| w[1] > w[0]));
        let unique: std::collections::BTreeSet<_> = seen.iter().collect();
        prop_assert_eq!(unique.len(), seen.len(), "an entry was returned twice");
        prop_assert_eq!(seen.len() as u64 + missed, pushed);
    }

    /// Reading the whole buffer at once returns exactly the retained entries
    /// with the payloads that were pushed.
    #[test]
    fn full_read_returns_retained_payloads(chunks in prop::collection::vec(prop::collection::vec(any::<u8>(), 0..32), 1..50)) {
        // Big enough that nothing is evicted.
        let mut log = LogBuffer::new(1 << 20).unwrap();
        for (ts, c) in (0u64..).zip(&chunks) {
            log.push_data(ts, Direction::Tx, Bytes::from(c.clone()));
        }
        let r = log.read(Cursor(0), usize::MAX).unwrap();
        prop_assert_eq!(r.missed, 0);
        let got: Vec<Vec<u8>> = r.entries.iter().map(|e| match &e.kind {
            EntryKind::Data { bytes, .. } => bytes.to_vec(),
            EntryKind::Marker { .. } => unreachable!(),
        }).collect();
        prop_assert_eq!(got, chunks);
        prop_assert_eq!(log.used(), r.entries.iter().map(|e| e.kind.payload_len() + ENTRY_OVERHEAD).sum::<usize>());
    }
}
