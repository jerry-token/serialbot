// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Decoded frames for the frames window: one decoder per session,
//! protocol and language, fed from the session's log as the window asks.
//!
//! A decoder starts at the beginning of the log, so choosing a protocol
//! after the data came still shows it. It keeps the newest [`MAX_FRAMES`]
//! frames, numbered from 0; a window reads the ones after the last number
//! it has. Bytes that wait past the protocol's gap without completing a
//! message come out as the window reads again, on the log's clock.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use serde::Serialize;
use serialbot_core::log::{Cursor, Direction, EntryKind};
use serialbot_core::session::Session;
use serialbot_decode::{Decoder, Frame, Lang};

/// Frames kept per decoder.
pub const MAX_FRAMES: usize = 50_000;

/// Log bytes read per step while catching up.
const READ_STEP: usize = 256 * 1024;

/// One frame with its number.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameDto {
    /// Its number, from 0, in the order frames ended.
    pub seq: u64,
    /// The frame.
    #[serde(flatten)]
    pub frame: Frame,
}

/// What a read gives the window.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FramesChunk {
    /// Frames from the asked number on, oldest first.
    pub frames: Vec<FrameDto>,
    /// The number to ask for next.
    pub next: u64,
    /// Frames after the asked number that are no longer kept.
    pub missed: u64,
}

struct Stream {
    decoder: Box<dyn Decoder>,
    cursor: Cursor,
    frames: VecDeque<FrameDto>,
    next_seq: u64,
    /// The last log entry's time and when it was read: "now" on the log's
    /// clock.
    last_seen: Option<(u64, Instant)>,
}

impl Stream {
    fn push(&mut self, frames: Vec<Frame>) {
        for frame in frames {
            self.frames.push_back(FrameDto {
                seq: self.next_seq,
                frame,
            });
            self.next_seq += 1;
            if self.frames.len() > MAX_FRAMES {
                self.frames.pop_front();
            }
        }
    }

    /// Feeds the log entries after the cursor, then gives back what waited
    /// past the gap.
    fn catch_up(&mut self, session: &Session) -> Result<(), String> {
        let mut out = Vec::new();
        loop {
            let chunk = session
                .read(self.cursor, READ_STEP)
                .map_err(|e| e.to_string())?;
            if chunk.entries.is_empty() {
                break;
            }
            for entry in &chunk.entries {
                if let EntryKind::Data { direction, bytes } = &entry.kind {
                    let direction = match direction {
                        Direction::Rx => serialbot_decode::Direction::Rx,
                        Direction::Tx => serialbot_decode::Direction::Tx,
                    };
                    self.decoder
                        .feed(direction, entry.timestamp_us, bytes, &mut out);
                    self.last_seen = Some((entry.timestamp_us, Instant::now()));
                }
            }
            self.cursor = chunk.next;
        }
        if let Some((at, seen)) = self.last_seen {
            let since = u64::try_from(seen.elapsed().as_micros()).unwrap_or(u64::MAX);
            self.decoder.idle(at.saturating_add(since), &mut out);
        }
        self.push(out);
        Ok(())
    }
}

/// The decoders of every session.
#[derive(Default)]
pub struct Decoders {
    streams: Mutex<HashMap<(u32, String, bool), Stream>>,
}

impl std::fmt::Debug for Decoders {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoders")
            .field("streams", &self.lock().len())
            .finish()
    }
}

impl Decoders {
    fn lock(&self) -> MutexGuard<'_, HashMap<(u32, String, bool), Stream>> {
        self.streams.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Up to `max` frames of `session` (id `id`) decoded as `protocol`
    /// (an id of [`serialbot_decode::PROTOCOLS`]), from number `after` on,
    /// with names in `lang`. Starts that decoder at the beginning of the
    /// log the first time. Fails for an unknown protocol or a bad log read.
    pub fn read(
        &self,
        id: u32,
        session: &Session,
        protocol: &str,
        lang: Lang,
        after: u64,
        max: usize,
    ) -> Result<FramesChunk, String> {
        let key = (id, protocol.to_ascii_lowercase(), lang == Lang::Zh);
        let mut streams = self.lock();
        if !streams.contains_key(&key) {
            let decoder = serialbot_decode::decoder(protocol, lang)
                .ok_or_else(|| format!("unknown protocol {protocol:?}"))?;
            streams.insert(
                key.clone(),
                Stream {
                    decoder,
                    cursor: Cursor(0),
                    frames: VecDeque::new(),
                    next_seq: 0,
                    last_seen: None,
                },
            );
        }
        let Some(stream) = streams.get_mut(&key) else {
            return Err("decoder missing".into());
        };
        stream.catch_up(session)?;
        let first = stream.frames.front().map_or(stream.next_seq, |f| f.seq);
        let from = after.max(first);
        let missed = first.saturating_sub(after);
        let skip = usize::try_from(from - first).unwrap_or(usize::MAX);
        let frames: Vec<FrameDto> = stream.frames.iter().skip(skip).take(max).cloned().collect();
        let next = frames.last().map_or(from.max(after), |f| f.seq + 1);
        Ok(FramesChunk {
            frames,
            next,
            missed,
        })
    }

    /// Forgets the session's decoders (its tab closed).
    pub fn remove(&self, id: u32) {
        self.lock().retain(|(session, _, _), _| *session != id);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use serialbot_core::transport::mock::{self, MockDevice};
    use serialbot_core::{SessionId, SessionOptions, SystemClock};
    use serialbot_decode::Status;

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

    fn wait_for_rx(s: &Session, bytes: usize) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let got: usize = s
                .read(Cursor(0), 1 << 20)
                .unwrap()
                .entries
                .iter()
                .map(|e| match &e.kind {
                    EntryKind::Data { bytes, .. } => bytes.len(),
                    EntryKind::Marker { .. } => 0,
                })
                .sum();
            if got >= bytes {
                return;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn frames_come_from_the_log_in_both_directions() {
        let (s, device) = session();
        let d = Decoders::default();
        assert!(format!("{d:?}").contains("Decoders"));
        // A request sent and its response received, before anyone asked.
        s.write(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x02, 0xC4, 0x0B])
            .unwrap();
        device.send(&[0x01, 0x03, 0x04, 0x00, 0xE7, 0x01, 0xA2, 0xCA, 0x2D]);
        wait_for_rx(&s, 17);
        let chunk = d.read(1, &s, "modbus-rtu", Lang::En, 0, 100).unwrap();
        assert_eq!(chunk.frames.len(), 2, "{chunk:#?}");
        assert_eq!(chunk.next, 2);
        assert_eq!(chunk.missed, 0);
        assert_eq!(chunk.frames[0].seq, 0);
        assert_eq!(
            chunk.frames[0].frame.direction,
            serialbot_decode::Direction::Tx
        );
        assert_eq!(chunk.frames[1].frame.summary, "slave 1 · 0=231 1=418");
        let json = serde_json::to_value(&chunk).unwrap();
        assert_eq!(json["frames"][1]["seq"], 1);
        assert_eq!(json["frames"][1]["status"], "ok");

        // Nothing new: nothing returned, the same next number.
        let again = d.read(1, &s, "modbus-rtu", Lang::En, 2, 100).unwrap();
        assert!(again.frames.is_empty());
        assert_eq!(again.next, 2);

        // Another protocol or language decodes the same log afresh.
        let zh = d.read(1, &s, "MODBUS-RTU", Lang::Zh, 0, 1).unwrap();
        assert_eq!(zh.frames.len(), 1);
        assert_eq!(zh.next, 1);
        assert_eq!(zh.frames[0].frame.name, "读保持寄存器 (03)");
        assert!(d.read(1, &s, "mavlink", Lang::En, 0, 1).is_err());

        d.remove(1);
        assert!(format!("{d:?}").contains("streams: 0"));
    }

    #[test]
    fn unfinished_bytes_come_out_after_the_gap() {
        let (s, device) = session();
        let d = Decoders::default();
        device.send(b"t12");
        wait_for_rx(&s, 3);
        let chunk = d.read(1, &s, "slcan", Lang::En, 0, 10).unwrap();
        assert!(chunk.frames.is_empty(), "the line may still end");
        std::thread::sleep(Duration::from_millis(250));
        let chunk = d.read(1, &s, "slcan", Lang::En, 0, 10).unwrap();
        assert_eq!(chunk.frames.len(), 1);
        assert_eq!(chunk.frames[0].frame.status, Status::Incomplete);
    }

    #[test]
    fn only_the_newest_frames_are_kept() {
        let (s, _device) = session();
        let mut stream = Stream {
            decoder: serialbot_decode::decoder("slcan", Lang::En).unwrap(),
            cursor: Cursor(0),
            frames: VecDeque::new(),
            next_seq: 0,
            last_seen: None,
        };
        let frame = |i: u64| Frame {
            timestamp_us: i,
            direction: serialbot_decode::Direction::Rx,
            bytes: vec![],
            name: String::new(),
            summary: String::new(),
            fields: vec![],
            status: Status::Ok,
        };
        stream.push(
            (0..u64::try_from(MAX_FRAMES).unwrap() + 5)
                .map(frame)
                .collect(),
        );
        assert_eq!(stream.frames.len(), MAX_FRAMES);
        assert_eq!(stream.frames[0].seq, 5);
        let d = Decoders::default();
        d.lock().insert((1, "slcan".into(), false), stream);
        let chunk = d.read(1, &s, "slcan", Lang::En, 2, 3).unwrap();
        assert_eq!(chunk.missed, 3);
        assert_eq!(
            chunk.frames.iter().map(|f| f.seq).collect::<Vec<_>>(),
            [5, 6, 7]
        );
        assert_eq!(chunk.next, 8);
        let end = u64::try_from(MAX_FRAMES).unwrap() + 5;
        let past = d.read(1, &s, "slcan", Lang::En, end + 10, 3).unwrap();
        assert!(past.frames.is_empty());
        assert_eq!(past.next, end + 10, "asking past the end keeps the number");
    }
}
