//! The real `serialbot-cli` program against virtual serial ports made by
//! `socat` (Linux only, like the core's socat tests; skipped without socat
//! unless `SERIALBOT_REQUIRE_SOCAT` is set).
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::print_stderr)]

use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use serialbot_core::transport::Connection;
use serialbot_core::transport::serial::{self, SerialConfig};

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

/// Runs the program with `args` in English.
fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_serialbot-cli"))
        .args(args)
        .env("LANG", "en_US.UTF-8")
        .env_remove("LC_ALL")
        .env_remove("LC_MESSAGES")
        .output()
        .unwrap()
}

/// A device on `path` that answers every line it receives with `OK`.
fn responder(path: &std::path::Path) -> std::thread::JoinHandle<Vec<u8>> {
    let mut device: Connection =
        serial::open(&SerialConfig::new(path.to_str().unwrap(), 115_200)).unwrap();
    std::thread::spawn(move || {
        let mut got = Vec::new();
        let mut buf = [0u8; 256];
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let n = device.reader.read(&mut buf).unwrap_or(0);
            got.extend_from_slice(&buf[..n]);
            if got.ends_with(b"\r\n") {
                device.control.write_all(b"OK\r\n").unwrap();
                break;
            }
        }
        got
    })
}

#[test]
fn send_expect_round_trip_over_a_real_port() {
    let Some(pair) = virtual_pair("cli-send") else {
        return;
    };
    let device = responder(&pair.b);
    let a = pair.a.to_str().unwrap();
    let out = cli(&[
        "send",
        a,
        "AT",
        "--expect",
        "OK",
        "--print",
        "--timeout",
        "5s",
    ]);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"OK\r\n");
    assert_eq!(device.join().unwrap(), b"AT\r\n");
}

#[test]
fn exit_codes_over_a_real_port() {
    let Some(pair) = virtual_pair("cli-codes") else {
        return;
    };
    let a = pair.a.to_str().unwrap();
    let silent = cli(&["send", a, "AT", "--expect", "OK", "--timeout", "200ms"]);
    assert_eq!(silent.status.code(), Some(1));
    let missing = cli(&["send", "/dev/serialbot-no-such-port", "AT"]);
    assert_eq!(missing.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&missing.stderr).starts_with("error: cannot open"));
    let usage = cli(&["send", a]);
    assert_eq!(usage.status.code(), Some(2));
    let ports = cli(&["ports", "--json"]);
    assert_eq!(ports.status.code(), Some(0));
}

#[test]
fn help_follows_the_system_language() {
    let zh = Command::new(env!("CARGO_BIN_EXE_serialbot-cli"))
        .arg("--help")
        .env("LANG", "zh_CN.UTF-8")
        .env_remove("LC_ALL")
        .env_remove("LC_MESSAGES")
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&zh.stdout).contains("用法"));
    let piped = cli(&["--color", "auto", "ports"]);
    assert!(!piped.stdout.contains(&0x1b), "no colors into a pipe");
}

#[test]
fn monitor_streams_until_a_match_or_ctrl_c() {
    let Some(pair) = virtual_pair("cli-monitor") else {
        return;
    };
    let a = pair.a.to_str().unwrap().to_owned();
    let mut device = serial::open(&SerialConfig::new(pair.b.to_str().unwrap(), 115_200)).unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_serialbot-cli"))
        .args(["monitor", &a, "--until", "ready", "--duration", "10s"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    device.control.write_all(b"boot\nready\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, b"boot\nready\n");

    // Ctrl+C ends it with success and closes the port.
    let child = Command::new(env!("CARGO_BIN_EXE_serialbot-cli"))
        .args(["monitor", &a])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(500));
    let pid = child.id().to_string();
    assert!(
        Command::new("kill")
            .args(["-INT", &pid])
            .status()
            .unwrap()
            .success()
    );
    let status = child.wait_with_output().unwrap().status;
    assert_eq!(status.code(), Some(0));
    let reopened = cli(&["send", &a, "x"]);
    assert_eq!(reopened.status.code(), Some(0), "the port was released");
}

#[test]
fn signal_reports_ports_without_modem_lines() {
    let Some(pair) = virtual_pair("cli-signal") else {
        return;
    };
    let a = pair.a.to_str().unwrap();
    let out = cli(&["signal", a, "dtr=0"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not supported"),
        "{out:?}"
    );
    // Only showing the levels works without modem lines.
    let out = cli(&["signal", a]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(out.stdout, b"DTR -  RTS -\n");
}
