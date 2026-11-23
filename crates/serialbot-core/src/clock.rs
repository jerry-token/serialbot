// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Source of timestamps, injectable so tests are deterministic.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Provides wall-clock timestamps in microseconds since the Unix epoch.
pub trait Clock: Send + Sync {
    /// Current time in microseconds since the Unix epoch.
    fn now_us(&self) -> u64;
}

/// The system wall clock.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_us(&self) -> u64 {
        // A clock set before 1970 reads as 0 rather than failing.
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX))
    }
}

/// A clock that only moves when told to; for tests.
#[derive(Debug, Default)]
pub struct ManualClock(AtomicU64);

impl ManualClock {
    /// Creates a clock reading `start_us`.
    pub fn new(start_us: u64) -> Self {
        Self(AtomicU64::new(start_us))
    }

    /// Moves the clock forward by `delta_us`.
    pub fn advance(&self, delta_us: u64) {
        self.0.fetch_add(delta_us, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_us(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_after_2020() {
        // 2020-01-01T00:00:00Z in microseconds.
        assert!(SystemClock.now_us() > 1_577_836_800_000_000);
    }

    #[test]
    fn manual_clock_moves_only_when_advanced() {
        let c = ManualClock::new(10);
        assert_eq!(c.now_us(), 10);
        c.advance(5);
        assert_eq!(c.now_us(), 15);
    }
}
