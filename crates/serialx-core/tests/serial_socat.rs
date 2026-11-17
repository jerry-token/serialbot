//! Exercises the real serial transport through a pair of virtual ports made
//! by `socat` (Linux and macOS).
//!
//! If `socat` is missing the tests are skipped, unless the environment
//! variable `SERIALX_REQUIRE_SOCAT` is set (CI sets it), in which case a
//! missing `socat` fails the run instead of silently skipping coverage.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::print_stderr)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serialx_core::transport::serial::{self, SerialConfig};
use serialx_core::transport::{Connection, TransportError, TransportKind};

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
        let p = std::env::temp_dir().join(format!("serialx-{tag}-{}-{nanos}", std::process::id()));
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
            std::env::var_os("SERIALX_REQUIRE_SOCAT").is_none(),
            "socat is required (SERIALX_REQUIRE_SOCAT is set) but not installed"
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
