// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! The interactive terminals (`serialbot-cli open` and `tui`) in a real pseudo
//! terminal, against socat virtual ports (Linux only; skipped without
//! socat unless `SERIALBOT_REQUIRE_SOCAT` is set). The screen is rebuilt
//! with a terminal emulator, so the tests check what a person would see.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::print_stderr)]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use expectrl::process::unix::WaitStatus;
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

/// `serialbot-cli open <port>` in an 80×24 pseudo terminal, with the
/// screen as a person would see it.
struct Term {
    session: expectrl::session::OsSession,
    screen: vt100::Parser,
}

impl Term {
    fn open(args: &[&str]) -> Self {
        Self::spawn("open", args, 80, 24)
    }

    fn spawn(subcommand: &str, args: &[&str], cols: u16, rows: u16) -> Self {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_serialbot-cli"));
        command
            .arg(subcommand)
            .args(args)
            .env("LANG", "en_US.UTF-8")
            .env("TERM", "xterm-256color");
        let mut session = expectrl::session::OsSession::spawn(command).unwrap();
        session
            .get_process_mut()
            .set_window_size(cols, rows)
            .unwrap();
        Self {
            session,
            screen: vt100::Parser::new(rows, cols, 0),
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        use std::io::Write as _;
        self.session.write_all(bytes).unwrap();
        self.session.flush().unwrap();
    }

    /// Reads output until the screen shows `text`.
    fn wait_for(&mut self, text: &str) {
        self.wait_until(text, |screen| screen.contains(text));
    }

    /// Reads output until `done` holds for the screen's text.
    fn wait_until(&mut self, text: &str, done: impl Fn(&str) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut buf = [0u8; 4096];
        while !done(&self.screen.screen().contents()) {
            assert!(
                Instant::now() < deadline,
                "{text:?} never showed; screen:\n{}",
                self.screen.screen().contents()
            );
            if let Ok(n) = self.session.try_read(&mut buf) {
                self.screen.process(&buf[..n]);
            } else {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    /// Waits for the program to end and returns its exit code.
    fn exit_code(&mut self) -> i32 {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut buf = [0u8; 4096];
        loop {
            let _ = self
                .session
                .try_read(&mut buf)
                .map(|n| self.screen.process(&buf[..n]));
            match self.session.get_process().status().unwrap() {
                WaitStatus::Exited(_, code) => return code,
                WaitStatus::StillAlive => {}
                other => panic!("ended with {other:?}"),
            }
            assert!(
                Instant::now() < deadline,
                "still running; screen:\n{}",
                self.screen.screen().contents()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

fn device(path: &std::path::Path) -> Connection {
    serial::open(&SerialConfig::new(path.to_str().unwrap(), 115_200)).unwrap()
}

/// Reads from `device` until `want` arrived.
fn receive(device: &mut Connection, want: &[u8]) {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut got = Vec::new();
    let mut buf = [0u8; 256];
    while !got.ends_with(want) {
        assert!(
            Instant::now() < deadline,
            "got {:?}",
            String::from_utf8_lossy(&got)
        );
        let n = device.reader.read(&mut buf).unwrap_or(0);
        got.extend_from_slice(&buf[..n]);
    }
}

#[test]
fn a_session_in_a_real_terminal() {
    let Some(pair) = virtual_pair("open-term") else {
        return;
    };
    let mut dev = device(&pair.b);
    let mut t = Term::open(&[pair.a.to_str().unwrap()]);
    t.wait_for("Ctrl+A Z help");

    // Typing and Enter send the line; the reply scrolls above the box.
    t.send(b"AT");
    t.wait_for("› AT");
    t.send(b"\r");
    // One CR per Enter, as minicom sends.
    receive(&mut dev, b"AT\r");
    dev.control.write_all(b"OK\r\nuart:~$ ").unwrap();
    // The input box is drawn again after the received text: wait for both.
    t.wait_until("uart:~$ and the input box below it", |screen| {
        let lines: Vec<&str> = screen.lines().collect();
        lines
            .iter()
            .position(|l| l.starts_with("uart:~$"))
            .is_some_and(|i| lines.get(i + 1).is_some_and(|l| l.starts_with('›')))
    });
    let screen = t.screen.screen().contents();
    let lines: Vec<&str> = screen.lines().collect();
    let ok = lines.iter().position(|l| *l == "OK").unwrap();
    assert_eq!(lines[ok - 1], "→ AT\\r", "{screen}");
    assert!(lines[ok + 1].starts_with("uart:~$"), "{screen}");
    assert!(lines[ok + 2].starts_with('›'), "{screen}");

    // A minicom key, then quitting with Ctrl+A X.
    t.send(b"\x01z");
    t.wait_for("Ctrl+A then: Z help");
    t.send(b"\x01x");
    assert_eq!(t.exit_code(), 0);
    assert!(
        !t.screen.screen().contents().contains('›'),
        "the input box is gone after quitting"
    );
    // The port was released.
    drop(device(&pair.a));
}

#[test]
fn raw_mode_and_termination() {
    let Some(pair) = virtual_pair("open-raw") else {
        return;
    };
    let mut dev = device(&pair.b);
    let mut t = Term::open(&[pair.a.to_str().unwrap(), "--raw"]);
    t.wait_for("raw mode");
    t.send(b"ls\r");
    receive(&mut dev, b"ls\r");
    t.send(b"\x1b[A");
    receive(&mut dev, b"\x1b[A");
    dev.control.write_all(b"\x1b[1mbold\x1b[0m\r\n").unwrap();
    t.wait_for("bold");
    // SIGTERM ends it cleanly: exit code 0 and the port free again.
    t.session
        .get_process_mut()
        .signal(expectrl::process::unix::Signal::SIGTERM)
        .unwrap();
    assert_eq!(t.exit_code(), 0);
    drop(device(&pair.a));
}

#[test]
fn a_lost_device_ends_the_session_with_4_when_asked() {
    let Some(pair) = virtual_pair("open-lost") else {
        return;
    };
    let mut t = Term::open(&[pair.a.to_str().unwrap(), "--no-reconnect"]);
    t.wait_for("Ctrl+A Z help");
    drop(pair);
    assert_eq!(t.exit_code(), 4);
}

#[test]
fn tui_in_a_real_terminal() {
    let Some(pair) = virtual_pair("tui-term") else {
        return;
    };
    let mut dev = device(&pair.b);
    let mut t = Term::spawn("tui", &[pair.a.to_str().unwrap()], 120, 30);
    t.wait_for("Sessions");
    assert!(t.screen.screen().alternate_screen(), "full screen");
    t.send(b"AT\r");
    receive(&mut dev, b"AT\r");
    dev.control.write_all(b"OK from the device\r\n").unwrap();
    t.wait_for("OK from the device");
    // A second session from the input box, then quitting.
    t.send(b"/open /nonexistent/port\r");
    t.wait_for("cannot open /nonexistent/port");
    t.send(b"\x01x");
    assert_eq!(t.exit_code(), 0);
    assert!(
        !t.screen.screen().alternate_screen(),
        "the screen is put back"
    );
    drop(device(&pair.a));
}

#[test]
fn tui_ends_cleanly_on_sigterm() {
    let Some(pair) = virtual_pair("tui-term") else {
        return;
    };
    let mut t = Term::spawn("tui", &[pair.a.to_str().unwrap()], 120, 30);
    t.wait_for("Sessions");
    t.session
        .get_process_mut()
        .signal(expectrl::process::unix::Signal::SIGTERM)
        .unwrap();
    assert_eq!(t.exit_code(), 0);
    assert!(!t.screen.screen().alternate_screen());
    drop(device(&pair.a));
}
