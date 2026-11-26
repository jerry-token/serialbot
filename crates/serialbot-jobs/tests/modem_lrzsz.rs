// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! XMODEM / YMODEM against lrzsz's receivers (`rx`, `rb`), a real
//! implementation on the other end of a socat virtual port.
//!
//! Linux only (see `serialbot-core/tests/serial_socat.rs`). Skipped when
//! socat or lrzsz is missing, unless `SERIALBOT_REQUIRE_LRZSZ` is set (CI
//! sets it).
#![cfg(target_os = "linux")]
#![allow(
    clippy::unwrap_used,
    clippy::print_stderr,
    clippy::cast_possible_truncation
)]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serialbot_core::transport::serial::{self, SerialConfig};
use serialbot_core::{Session, SessionId, SessionOptions, SystemClock};
use serialbot_jobs::xmodem::{self, Modem, SessionLink};

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("serialbot-lrzsz-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        if let Ok(log) = std::fs::read_to_string(self.0.join("receiver.log"))
            && !log.trim().is_empty()
        {
            eprintln!("receiver said:\n{log}");
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Peer(Child);

impl Drop for Peer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn have(program: &str, arg: &str) -> bool {
    Command::new(program)
        .arg(arg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// A virtual port whose other end runs `receiver` (in `dir`), or `None`
/// when the tools are missing and not required.
fn receiver(dir: &Dir, receiver: &str) -> Option<(Peer, PathBuf)> {
    if !(have("socat", "-V") && have("rx", "--version") && have("rb", "--version")) {
        assert!(
            std::env::var_os("SERIALBOT_REQUIRE_LRZSZ").is_none(),
            "socat and lrzsz are required (SERIALBOT_REQUIRE_LRZSZ is set)"
        );
        eprintln!("skipping: socat or lrzsz not installed");
        return None;
    }
    let port = dir.0.join("port");
    let child = Command::new("socat")
        .arg(format!("pty,raw,echo=0,link={}", port.display()))
        .arg(format!(
            "SYSTEM:cd {} && exec {receiver} 2>receiver.log,pty,raw,echo=0",
            dir.0.display()
        ))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !port.exists() {
        assert!(Instant::now() < deadline, "socat did not create the port");
        std::thread::sleep(Duration::from_millis(20));
    }
    Some((Peer(child), port))
}

fn session(port: &std::path::Path) -> Arc<Session> {
    let connection = serial::open(&SerialConfig::new(port.to_str().unwrap(), 115_200)).unwrap();
    Arc::new(
        Session::start(
            SessionId(1),
            connection,
            Arc::new(SystemClock),
            SessionOptions::default(),
        )
        .unwrap(),
    )
}

fn firmware(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 253) as u8).collect()
}

fn wait_for_file(path: &std::path::Path, len: usize) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(data) = std::fs::read(path)
            && data.len() >= len
        {
            return data;
        }
        assert!(Instant::now() < deadline, "{} not written", path.display());
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Sends `len` bytes with `modem` to `receiver_cmd`; `None` when skipped.
fn transfer(name: &str, receiver_cmd: &str, modem: &str, len: usize) -> Option<(Dir, Vec<u8>)> {
    let dir = Dir::new(name);
    let data = firmware(len);
    let (_peer, port) = receiver(&dir, receiver_cmd)?;
    let session = session(&port);
    let mut link = SessionLink::new(&session, || false);
    xmodem::send(
        &mut link,
        Modem::parse(modem).unwrap(),
        "fw.bin",
        &data,
        &mut |_| {},
    )
    .unwrap();
    session.close();
    Some((dir, data))
}

#[test]
fn xmodem_crc_to_lrzsz_rx() {
    let Some((dir, data)) = transfer("xcrc", "rx -c -q out.bin", "xmodem", 5000) else {
        return;
    };
    let got = wait_for_file(&dir.0.join("out.bin"), data.len());
    // XMODEM pads the last block; the file holds whole blocks.
    assert_eq!(&got[..data.len()], &data[..]);
    assert_eq!(got.len() % 128, 0);
}

#[test]
fn xmodem_checksum_to_lrzsz_rx() {
    let Some((dir, data)) = transfer("xsum", "rx -q out.bin", "xmodem", 1000) else {
        return;
    };
    let got = wait_for_file(&dir.0.join("out.bin"), data.len());
    assert_eq!(&got[..data.len()], &data[..]);
}

#[test]
fn xmodem_1k_to_lrzsz_rx() {
    let Some((dir, data)) = transfer("x1k", "rx -c -q out.bin", "xmodem-1k", 10_000) else {
        return;
    };
    let got = wait_for_file(&dir.0.join("out.bin"), data.len());
    assert_eq!(&got[..data.len()], &data[..]);
}

#[test]
fn ymodem_to_lrzsz_rb_keeps_name_and_exact_size() {
    for (tag, modem) in [("y1k", "ymodem"), ("y128", "ymodem-128")] {
        let Some((dir, data)) = transfer(tag, "rb -q", modem, 70_000 + 77) else {
            return;
        };
        let got = wait_for_file(&dir.0.join("fw.bin"), data.len());
        // YMODEM's header carries the size: no padding is kept.
        assert_eq!(got, data, "{modem}");
    }
}
