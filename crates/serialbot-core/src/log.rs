//! Timestamped, bounded log of everything that crossed a transport.
//!
//! Every received chunk, every sent chunk and every marker becomes an
//! [`Entry`] with a monotonically increasing sequence number. Readers keep a
//! [`Cursor`] (the next sequence number they want) and read incrementally, so
//! the UI, scripts, the AI agent and the MCP server can all consume the same
//! stream at their own pace without copying it.
//!
//! The buffer is bounded by a byte budget. When it is full the oldest entries
//! are evicted; a reader whose cursor points at evicted entries is told how
//! many it missed instead of silently skipping them.

use std::collections::VecDeque;

use bytes::Bytes;

/// Fixed bookkeeping cost charged per entry on top of its payload.
///
/// Without it, a flood of one-byte chunks could hold an unbounded number of
/// entries while staying under the payload budget.
pub const ENTRY_OVERHEAD: usize = 32;

/// Which way the data travelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Received from the device.
    Rx,
    /// Sent to the device.
    Tx,
}

/// What an entry holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    /// Raw bytes that crossed the transport.
    Data {
        /// Direction of the bytes.
        direction: Direction,
        /// The bytes themselves, exactly as they crossed the wire.
        bytes: Bytes,
    },
    /// A marker inserted by a user, script or agent (for example "reset").
    Marker {
        /// Free-form label of the marker.
        label: String,
    },
}

impl EntryKind {
    /// Payload size of this entry in bytes (data length or label length).
    pub fn payload_len(&self) -> usize {
        match self {
            Self::Data { bytes, .. } => bytes.len(),
            Self::Marker { label } => label.len(),
        }
    }
}

/// Who caused a log entry: the person at the app (or the device, for
/// received data) or an AI agent through MCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Actor {
    /// The user, the app, or the device.
    #[default]
    User,
    /// An AI agent.
    Ai,
}

/// One record in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Sequence number, unique and strictly increasing within one buffer.
    pub seq: u64,
    /// Wall-clock time in microseconds since the Unix epoch, as supplied by
    /// the caller of [`LogBuffer::push`].
    pub timestamp_us: u64,
    /// Content of the entry.
    pub kind: EntryKind,
    /// Who caused it, so the UI can show what an agent did.
    pub actor: Actor,
}

impl Entry {
    /// Budget cost of this entry: payload plus [`ENTRY_OVERHEAD`].
    pub fn cost(&self) -> usize {
        self.kind.payload_len().saturating_add(ENTRY_OVERHEAD)
    }
}

/// Position of a reader in the log: the sequence number of the next entry it
/// wants to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Cursor(pub u64);

/// Result of an incremental read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadResult {
    /// Entries in sequence order.
    pub entries: Vec<Entry>,
    /// Cursor to pass to the next read.
    pub next: Cursor,
    /// Number of entries the reader wanted but that were already evicted.
    pub missed: u64,
}

/// Errors returned by [`LogBuffer`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LogError {
    /// The cursor points past the newest entry, so it was not produced by
    /// this buffer.
    #[error("cursor {cursor} is ahead of the log (next sequence number is {next})")]
    CursorAhead {
        /// The offending cursor.
        cursor: u64,
        /// The next sequence number the buffer will assign.
        next: u64,
    },
    /// A buffer cannot be created with a zero byte budget.
    #[error("log capacity must be greater than zero")]
    ZeroCapacity,
}

/// Bounded log of timestamped entries.
///
/// The newest entry is always retained, even if it alone exceeds the budget;
/// apart from that, the total [`Entry::cost`] never exceeds the capacity.
#[derive(Debug)]
pub struct LogBuffer {
    entries: VecDeque<Entry>,
    capacity: usize,
    used: usize,
    next_seq: u64,
}

impl LogBuffer {
    /// Creates an empty buffer that holds at most `capacity` bytes of
    /// entries (payload plus per-entry overhead).
    pub fn new(capacity: usize) -> Result<Self, LogError> {
        if capacity == 0 {
            return Err(LogError::ZeroCapacity);
        }
        Ok(Self {
            entries: VecDeque::new(),
            capacity,
            used: 0,
            next_seq: 0,
        })
    }

    /// Byte budget of the buffer.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Bytes currently charged against the budget.
    pub fn used(&self) -> usize {
        self.used
    }

    /// Number of retained entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the buffer holds no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Cursor of the oldest retained entry (equal to [`Self::end`] when
    /// empty).
    pub fn start(&self) -> Cursor {
        Cursor(self.entries.front().map_or(self.next_seq, |e| e.seq))
    }

    /// Cursor just past the newest entry: a reader holding it is up to date.
    pub fn end(&self) -> Cursor {
        Cursor(self.next_seq)
    }

    /// Appends an entry and returns its sequence number, evicting the
    /// oldest entries if the budget would be exceeded.
    pub fn push(&mut self, timestamp_us: u64, kind: EntryKind) -> u64 {
        self.push_as(timestamp_us, kind, Actor::User)
    }

    /// Like [`Self::push`], recording who caused the entry.
    pub fn push_as(&mut self, timestamp_us: u64, kind: EntryKind, actor: Actor) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        let entry = Entry {
            seq,
            timestamp_us,
            kind,
            actor,
        };
        self.used += entry.cost();
        self.entries.push_back(entry);
        self.evict();
        seq
    }

    /// Convenience wrapper around [`Self::push`] for received or sent bytes.
    pub fn push_data(&mut self, timestamp_us: u64, direction: Direction, bytes: Bytes) -> u64 {
        self.push(timestamp_us, EntryKind::Data { direction, bytes })
    }

    /// Convenience wrapper around [`Self::push`] for markers.
    pub fn push_marker(&mut self, timestamp_us: u64, label: impl Into<String>) -> u64 {
        self.push(
            timestamp_us,
            EntryKind::Marker {
                label: label.into(),
            },
        )
    }

    /// Reads entries starting at `cursor`.
    ///
    /// At most `max_payload` bytes of payload are returned, except that the
    /// first available entry is always returned so a reader can make
    /// progress even through an entry larger than its limit.
    pub fn read(&self, cursor: Cursor, max_payload: usize) -> Result<ReadResult, LogError> {
        if cursor.0 > self.next_seq {
            return Err(LogError::CursorAhead {
                cursor: cursor.0,
                next: self.next_seq,
            });
        }
        let start = self.start().0;
        let missed = start.saturating_sub(cursor.0);
        let from = cursor.0.max(start);
        // Sequence numbers in the deque are contiguous, so the offset of
        // `from` is a plain subtraction. `from <= next_seq`, hence
        // `offset <= len` and `range` cannot panic; `range` also starts in
        // O(1) instead of walking the skipped entries.
        let offset =
            usize::try_from(from - start).map_or(self.entries.len(), |o| o.min(self.entries.len()));

        let mut entries = Vec::new();
        let mut payload = 0usize;
        for entry in self.entries.range(offset..) {
            let len = entry.kind.payload_len();
            if !entries.is_empty() && payload.saturating_add(len) > max_payload {
                break;
            }
            payload = payload.saturating_add(len);
            entries.push(entry.clone());
        }
        let next = Cursor(entries.last().map_or(from, |e| e.seq + 1));
        Ok(ReadResult {
            entries,
            next,
            missed,
        })
    }

    fn evict(&mut self) {
        while self.used > self.capacity && self.entries.len() > 1 {
            if let Some(old) = self.entries.pop_front() {
                self.used -= old.cost();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(s: &'static str) -> EntryKind {
        EntryKind::Data {
            direction: Direction::Rx,
            bytes: Bytes::from_static(s.as_bytes()),
        }
    }

    fn payloads(r: &ReadResult) -> Vec<Vec<u8>> {
        r.entries
            .iter()
            .map(|e| match &e.kind {
                EntryKind::Data { bytes, .. } => bytes.to_vec(),
                EntryKind::Marker { label } => label.as_bytes().to_vec(),
            })
            .collect()
    }

    #[test]
    fn zero_capacity_is_rejected() {
        assert_eq!(LogBuffer::new(0).unwrap_err(), LogError::ZeroCapacity);
    }

    #[test]
    fn empty_buffer_reads_nothing() {
        let log = LogBuffer::new(1024).unwrap();
        let r = log.read(Cursor(0), usize::MAX).unwrap();
        assert_eq!(r.entries, []);
        assert_eq!(r.next, Cursor(0));
        assert_eq!(r.missed, 0);
        assert_eq!(log.start(), log.end());
        assert!(log.is_empty());
        assert_eq!(log.used(), 0);
        assert_eq!(log.capacity(), 1024);
    }

    #[test]
    fn push_assigns_contiguous_sequence_numbers() {
        let mut log = LogBuffer::new(1024).unwrap();
        assert_eq!(log.push(10, data("a")), 0);
        assert_eq!(log.push(11, data("b")), 1);
        assert_eq!(log.push_marker(12, "reset"), 2);
        assert_eq!(log.end(), Cursor(3));
        assert_eq!(log.len(), 3);
    }

    #[test]
    fn incremental_reads_see_every_entry_once() {
        let mut log = LogBuffer::new(1024).unwrap();
        log.push(1, data("ab"));
        let first = log.read(Cursor(0), usize::MAX).unwrap();
        assert_eq!(payloads(&first), vec![b"ab".to_vec()]);

        log.push(2, data("cd"));
        log.push(3, data("ef"));
        let second = log.read(first.next, usize::MAX).unwrap();
        assert_eq!(payloads(&second), vec![b"cd".to_vec(), b"ef".to_vec()]);

        let third = log.read(second.next, usize::MAX).unwrap();
        assert_eq!(third.entries, []);
        assert_eq!(third.next, second.next);
    }

    #[test]
    fn timestamps_and_directions_are_preserved() {
        let mut log = LogBuffer::new(1024).unwrap();
        log.push_data(42, Direction::Tx, Bytes::from_static(b"AT\r\n"));
        let r = log.read(Cursor(0), usize::MAX).unwrap();
        assert_eq!(r.entries[0].timestamp_us, 42);
        assert_eq!(
            r.entries[0].kind,
            EntryKind::Data {
                direction: Direction::Tx,
                bytes: Bytes::from_static(b"AT\r\n"),
            }
        );
    }

    #[test]
    fn max_payload_limits_a_read_but_always_makes_progress() {
        let mut log = LogBuffer::new(1024).unwrap();
        log.push(1, data("aaaa"));
        log.push(2, data("bb"));
        log.push(3, data("cc"));

        // The first entry is larger than the limit but is still returned.
        let r = log.read(Cursor(0), 1).unwrap();
        assert_eq!(payloads(&r), vec![b"aaaa".to_vec()]);
        assert_eq!(r.next, Cursor(1));

        // Two entries of two bytes fit exactly into a limit of four.
        let r = log.read(r.next, 4).unwrap();
        assert_eq!(payloads(&r), vec![b"bb".to_vec(), b"cc".to_vec()]);
    }

    #[test]
    fn eviction_keeps_usage_within_capacity() {
        let cap = 3 * (ENTRY_OVERHEAD + 2);
        let mut log = LogBuffer::new(cap).unwrap();
        for i in 0..10 {
            log.push(i, data("xx"));
            assert!(log.used() <= cap);
        }
        assert_eq!(log.len(), 3);
        assert_eq!(log.start(), Cursor(7));
    }

    #[test]
    fn reader_behind_eviction_is_told_how_much_it_missed() {
        let cap = 2 * (ENTRY_OVERHEAD + 1);
        let mut log = LogBuffer::new(cap).unwrap();
        for i in 0..5 {
            log.push(i, data("x"));
        }
        let r = log.read(Cursor(0), usize::MAX).unwrap();
        assert_eq!(r.missed, 3);
        assert_eq!(r.entries.first().map(|e| e.seq), Some(3));
        assert_eq!(r.next, Cursor(5));
    }

    #[test]
    fn oversized_entry_is_kept_alone() {
        let mut log = LogBuffer::new(ENTRY_OVERHEAD + 4).unwrap();
        log.push(1, data("ab"));
        log.push(2, data("this is far longer than the budget"));
        assert_eq!(log.len(), 1);
        assert_eq!(log.start(), Cursor(1));
        // The next push evicts the oversized entry again.
        log.push(3, data("ok"));
        assert_eq!(log.len(), 1);
        assert!(log.used() <= log.capacity());
    }

    #[test]
    fn cursor_ahead_of_log_is_an_error() {
        let mut log = LogBuffer::new(1024).unwrap();
        log.push(1, data("a"));
        assert_eq!(
            log.read(Cursor(5), usize::MAX).unwrap_err(),
            LogError::CursorAhead { cursor: 5, next: 1 }
        );
        // The end cursor itself is valid and simply reads nothing.
        assert_eq!(log.read(log.end(), 10).unwrap().entries, []);
    }

    #[test]
    fn markers_count_against_the_budget() {
        let mut log = LogBuffer::new(ENTRY_OVERHEAD + 5).unwrap();
        log.push_marker(1, "reset");
        assert_eq!(log.used(), ENTRY_OVERHEAD + 5);
        log.push_marker(2, "boot");
        assert_eq!(log.len(), 1);
    }
}
