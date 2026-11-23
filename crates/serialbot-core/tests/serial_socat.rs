// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Exercises the real serial transport through a pair of virtual ports made
//! by `socat`.
//!
//! Linux only: on macOS `serialport` sets the baud rate with the
//! `IOSSIOSPEED` ioctl (that is how it supports non-standard rates), which
//! pseudo-terminals reject with `ENOTTY`. Real USB adapters support it, so
//! macOS serial I/O is verified with hardware instead (see CLAUDE.md).
//!
//! If `socat` is missing the tests are skipped, unless the environment
//! variable `SERIALBOT_REQUIRE_SOCAT` is set (CI sets it), in which case a
//! missing `socat` fails the run instead of silently skipping coverage.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::print_stderr)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serialbot_core::transport::serial::{self, SerialConfig};
use serialbot_core::transport::{Connection, TransportError, TransportKind};

struct VirtualPair {
    child: Child,
    a: PathBuf,
    b: PathBuf,
    _dir: TempDir,
}

impl Drop for VirtualPair {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Minimal temp dir so the tests need no extra dependency.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p =
            std::env::temp_dir().join(format!("serialbot-{tag}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn socat_available() -> bool {
    Command::new("socat")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Starts socat, or returns `None` when it is missing and not required.
fn virtual_pair(tag: &str) -> Option<VirtualPair> {
    if !socat_available() {
        assert!(
            std::env::var_os("SERIALBOT_REQUIRE_SOCAT").is_none(),
            "socat is required (SERIALBOT_REQUIRE_SOCAT is set) but not installed"
        );
        eprintln!("skipping: socat not installed");
        return None;
    }
    let dir = TempDir::new(tag);
    let a = dir.0.join("a");
    let b = dir.0.join("b");
    let child = Command::new("socat")
        .arg(format!("pty,raw,echo=0,link={}", a.display()))
        .arg(format!("pty,raw,echo=0,link={}", b.display()))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !(a.exists() && b.exists()) {
        assert!(Instant::now() < deadline, "socat did not create the ports");
        std::thread::sleep(Duration::from_millis(20));
    }
    Some(VirtualPair {
        child,
        a,
        b,
        _dir: dir,
    })
}

fn open(path: &Path) -> Connection {
    serial::open(&SerialConfig::new(path.to_str().unwrap(), 115_200)).unwrap()
}

/// Reads until `want` bytes arrived or `timeout` passed.
fn read_exact(conn: &mut Connection, want: usize, timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    let mut out = Vec::with_capacity(want);
    let mut buf = vec![0u8; 4096];
    while out.len() < want {
        assert!(
            Instant::now() < deadline,
            "timed out after {} of {want} bytes",
            out.len()
        );
        let n = conn.reader.read(&mut buf).unwrap();
        out.extend_from_slice(&buf[..n]);
    }
    out
}

#[test]
fn bytes_round_trip_in_both_directions() {
    let Some(pair) = virtual_pair("roundtrip") else {
        return;
    };
    let mut a = open(&pair.a);
    let mut b = open(&pair.b);
    assert_eq!(a.info.kind, TransportKind::Serial);

    a.control.write_all(b"AT\r\n").unwrap();
    assert_eq!(read_exact(&mut b, 4, Duration::from_secs(5)), b"AT\r\n");

    b.control.write_all(b"OK\r\n").unwrap();
    assert_eq!(read_exact(&mut a, 4, Duration::from_secs(5)), b"OK\r\n");
}

#[test]
fn large_binary_payload_arrives_intact() {
    let Some(pair) = virtual_pair("bulk") else {
        return;
    };
    let mut a = open(&pair.a);
    let mut b = open(&pair.b);
    // Every byte value, repeated: catches any text-mode translation.
    let payload: Vec<u8> = (0..=255u8).cycle().take(64 * 1024).collect();

    let writer = std::thread::spawn(move || {
        for chunk in payload.chunks(1024) {
            a.control.write_all(chunk).unwrap();
        }
        (a, payload)
    });
    let got = read_exact(&mut b, 64 * 1024, Duration::from_secs(20));
    let (_a, payload) = writer.join().unwrap();
    assert_eq!(got, payload);
}

#[test]
fn read_without_data_times_out_with_zero() {
    let Some(pair) = virtual_pair("timeout") else {
        return;
    };
    let mut a = open(&pair.a);
    let mut buf = [0u8; 16];
    let started = Instant::now();
    assert_eq!(a.reader.read(&mut buf).unwrap(), 0);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(a.reader.read(&mut []).unwrap(), 0);
}

#[test]
fn second_open_of_the_same_port_is_busy() {
    let Some(pair) = virtual_pair("busy") else {
        return;
    };
    let _first = open(&pair.a);
    let second = serial::open(&SerialConfig::new(pair.a.to_str().unwrap(), 115_200));
    assert!(matches!(second, Err(TransportError::Busy(_))), "{second:?}");
}

#[test]
fn closing_the_peer_is_reported_as_disconnected() {
    let Some(mut pair) = virtual_pair("hangup") else {
        return;
    };
    let mut a = open(&pair.a);
    pair.child.kill().unwrap();
    pair.child.wait().unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut buf = [0u8; 16];
    loop {
        match a.reader.read(&mut buf) {
            Err(TransportError::Disconnected) => break,
            Ok(_) => assert!(Instant::now() < deadline, "hang-up was never reported"),
            Err(e) => panic!("unexpected error: {e:?}"),
        }
    }
    // Once disconnected, the reader stays disconnected.
    assert!(matches!(
        a.reader.read(&mut buf),
        Err(TransportError::Disconnected)
    ));
}

#[test]
fn live_baud_change_and_break_work_on_a_pty() {
    let Some(pair) = virtual_pair("ctl") else {
        return;
    };
    let mut a = open(&pair.a);
    let mut b = open(&pair.b);
    a.control.set_baud_rate(921_600).unwrap();
    a.control.send_break(Duration::from_millis(10)).unwrap();
    assert!(matches!(
        a.control.set_baud_rate(0),
        Err(TransportError::InvalidConfig(_))
    ));
    // The port keeps working after the changes.
    a.control.write_all(b"still alive").unwrap();
    assert_eq!(
        read_exact(&mut b, 11, Duration::from_secs(5)),
        b"still alive"
    );
}

#[test]
fn modem_lines_on_a_pty_are_reported_as_unsupported() {
    let Some(pair) = virtual_pair("lines") else {
        return;
    };
    let mut a = open(&pair.a);
    assert!(matches!(
        a.control.set_dtr(true),
        Err(TransportError::Unsupported(_))
    ));
    assert!(matches!(
        a.control.set_rts(false),
        Err(TransportError::Unsupported(_))
    ));
    assert!(matches!(
        a.control.line_states(),
        Err(TransportError::Unsupported(_))
    ));
}

#[test]
fn initial_line_levels_are_ignored_on_a_pty() {
    let Some(pair) = virtual_pair("initial-lines") else {
        return;
    };
    let mut config = SerialConfig::new(pair.a.to_str().unwrap(), 115_200);
    config.dtr_on_open = Some(false);
    config.rts_on_open = Some(false);
    let mut a = serial::open(&config).unwrap();
    let mut b = open(&pair.b);
    a.control.write_all(b"hi").unwrap();
    assert_eq!(read_exact(&mut b, 2, Duration::from_secs(5)), b"hi");
}

/// Byte `i` of the stress stream: depends on the position, so a lost,
/// repeated or reordered byte shows up as a mismatch.
fn pattern(i: u64) -> u8 {
    // The top byte of a multiplicative hash of the position.
    i.wrapping_mul(0x9E37_79B9_7F4A_7C15).to_be_bytes()[0]
}

/// Sustained-throughput test (Spike 1): a peer sends 2 Mbit/s (250 kB/s)
/// into a session for `SERIALBOT_STRESS_SECS` seconds (3 by default; the
/// stress workflow runs an hour), while a consumer reads the log like the
/// window does: 256 KiB per read, every 16 ms. Every byte must arrive, in
/// order, with no log entry evicted before it was read.
#[test]
fn sustained_2_mbps_is_received_without_loss() {
    use serialbot_core::{Cursor, Direction, EntryKind, SessionManager};

    const RATE: u64 = 250_000; // bytes per second
    const TICK_MS: u64 = 10;
    const TICK: Duration = Duration::from_millis(TICK_MS);
    let Some(pair) = virtual_pair("stress") else {
        return;
    };
    let secs: u64 = std::env::var("SERIALBOT_STRESS_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let total = RATE * secs;

    let manager = SessionManager::with_serial_ports();
    let session = manager
        .open(
            &SerialConfig::new(pair.a.to_str().unwrap(), 2_000_000),
            None,
        )
        .unwrap();
    let mut peer = open(&pair.b);

    let writer = std::thread::spawn(move || {
        let start = Instant::now();
        let per_tick = RATE * TICK_MS / 1000;
        let mut sent = 0u64;
        let mut chunk = Vec::with_capacity(per_tick as usize);
        while sent < total {
            // Paced against the start, so short stalls are made up.
            let due = start + TICK * u32::try_from(sent / per_tick).unwrap();
            if let Some(wait) = due.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            let n = per_tick.min(total - sent);
            chunk.clear();
            chunk.extend((sent..sent + n).map(pattern));
            peer.control.write_all(&chunk).unwrap();
            sent += n;
        }
        start.elapsed()
    });

    let deadline = Instant::now() + Duration::from_secs(secs * 2 + 30);
    let mut cursor = Cursor(0);
    let mut received = 0u64;
    let mut reads = 0u64;
    let mut slowest_read = Duration::ZERO;
    while received < total {
        assert!(
            Instant::now() < deadline,
            "only {received} of {total} bytes arrived"
        );
        std::thread::sleep(Duration::from_millis(16));
        let started = Instant::now();
        let chunk = session.read(cursor, 256 * 1024).unwrap();
        slowest_read = slowest_read.max(started.elapsed());
        reads += 1;
        assert_eq!(
            chunk.missed, 0,
            "log entries were evicted before being read"
        );
        for entry in &chunk.entries {
            if let EntryKind::Data {
                direction: Direction::Rx,
                bytes,
            } = &entry.kind
            {
                for (k, &b) in bytes.iter().enumerate() {
                    let at = received + k as u64;
                    assert_eq!(b, pattern(at), "byte {at} is wrong (lost or out of order)");
                }
                received += bytes.len() as u64;
            }
        }
        cursor = chunk.next;
    }
    let sending = writer.join().unwrap();
    assert_eq!(received, total, "more bytes arrived than were sent");
    #[allow(clippy::cast_precision_loss)]
    let rate = total as f64 * 8.0 / sending.as_secs_f64() / 1e6;
    eprintln!(
        "stress: {total} bytes in {sending:.1?} ({rate:.2} Mbit/s), {reads} reads, \
         slowest read {slowest_read:?}"
    );
    assert!(rate > 1.9, "the peer only managed {rate:.2} Mbit/s");
    manager.close_all();
}
