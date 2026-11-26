// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! `send`: open the port, send once, optionally wait for a reply, close.
//!
//! Sending and waiting happen within one open, so a reply that comes right
//! after the data is never missed (a separate `send` then `monitor` would
//! reopen the port and could lose it).

use std::sync::Arc;
use std::time::Duration;

use serialbot_core::log::{Cursor, Direction, EntryKind};
use serialbot_core::manager::ManagerError;
use serialbot_core::session::{Session, SessionError};
use serialbot_core::transport::TransportError;
use serialbot_core::{SessionManager, encoding, hex};

use crate::Failure;
use serialbot_jobs::xmodem::{self, ModemError, SessionLink};

use crate::args::{FileProtocol, PortArgs, SendArgs};
use crate::exit::Exit;
use crate::i18n::Strings;
use crate::output::Output;

/// How many trailing received bytes a reply pattern may span.
const MATCH_WINDOW: usize = 64 * 1024;

/// Bytes read from the log per step when printing what arrived.
const READ_STEP: usize = 64 * 1024;

/// Sends the data. Exit codes: 0 sent (and the reply matched, with
/// `--expect`); 1 no match before `--timeout`; 2 bad data (hex, encoding,
/// file); 3 the port cannot be opened; 4 the device went away. With
/// `--print`, received bytes go to standard output unchanged: up to the
/// match, or everything within `--timeout` without `--expect`.
///
/// With `--protocol` the file goes to a bootloader with XMODEM / YMODEM:
/// 0 delivered; 1 the receiver did not start within `--timeout` (60s),
/// refused blocks or cancelled; 4 the device went away before the data was
/// in.
pub fn run(args: &SendArgs, manager: &SessionManager, out: &mut Output<'_>) -> Result<(), Failure> {
    let s = out.strings();
    let data = payload(args, s)?;
    let session = open(manager, &args.port, s)?;
    let result = match args.protocol {
        Some(protocol) => transfer(&session, args, protocol, &data, out),
        None => exchange(&session, args, &data, out),
    };
    // The port is the user's again as soon as we are done.
    let _ = manager.remove(session.id());
    result
}

/// The bytes to send: text (encoded, then checksum, then line ending), hex
/// or a file's content (then checksum).
fn payload(args: &SendArgs, s: &Strings) -> Result<Vec<u8>, Failure> {
    let usage = |message: String| Failure::new(Exit::Usage, message);
    let mut data = if let Some(text) = &args.text {
        encoding::encode_text(text, &args.encoding).map_err(|e| usage(e.to_string()))?
    } else if let Some(h) = &args.hex {
        hex::parse_hex(h).map_err(usage)?
    } else if let Some(path) = &args.file {
        std::fs::read(path)
            .map_err(|e| usage(format!("{} {}: {e}", s.cannot_read, path.display())))?
    } else {
        Vec::new()
    };
    if let Some(checksum) = args.checksum {
        let check = checksum.bytes(&data);
        data.extend(check);
    }
    if args.text.is_some() {
        data.extend_from_slice(args.ending.bytes());
    }
    if data.is_empty() {
        return Err(usage(s.empty_data.to_owned()));
    }
    Ok(data)
}

/// Opens the port; failures exit with 3 (2 for invalid settings).
pub(crate) fn open(
    manager: &SessionManager,
    port: &PortArgs,
    s: &Strings,
) -> Result<Arc<Session>, Failure> {
    manager.open(&port.config(), None).map_err(|e| {
        let exit = match &e {
            ManagerError::Session(SessionError::Transport(TransportError::InvalidConfig(_))) => {
                Exit::Usage
            }
            _ => Exit::Busy,
        };
        Failure::new(exit, format!("{} {}: {e}", s.cannot_open, port.port))
    })
}

/// A session error while in use: the device went away (exit 4).
pub(crate) fn lost(e: &SessionError, port: &str, s: &Strings) -> Failure {
    Failure::new(
        Exit::Disconnected,
        format!("{} ({port}): {e}", s.disconnected),
    )
}

/// The default `--timeout`.
const TIMEOUT: Duration = Duration::from_secs(5);

/// `--protocol`: the file to a bootloader.
fn transfer(
    session: &Session,
    args: &SendArgs,
    protocol: FileProtocol,
    data: &[u8],
    out: &mut Output<'_>,
) -> Result<(), Failure> {
    let s = out.strings();
    let modem = protocol.modem(args.timeout.unwrap_or(Duration::from_secs(60)));
    let name = args
        .file
        .as_ref()
        .and_then(|p| p.file_name())
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    out.error_note(&format!("{}: {} {name}", modem.name(), s.modem_waiting));
    let mut link = SessionLink::new(session, || false);
    match xmodem::send(&mut link, modem, &name, data, &mut |_| {}) {
        Ok(done) => {
            let closed = if done.closed_early {
                format!("; {}", s.modem_closed_early)
            } else {
                String::new()
            };
            out.error_note(&format!(
                "{} {name} ({} B, {}){closed}",
                s.modem_sent,
                data.len(),
                modem.name()
            ));
            Ok(())
        }
        Err(ModemError::Link(e)) => Err(Failure::new(
            Exit::Disconnected,
            format!("{} ({}): {e}", s.disconnected, args.port.port),
        )),
        Err(e) => Err(Failure::new(
            Exit::Timeout,
            format!("{}: {e}", modem.name()),
        )),
    }
}

fn exchange(
    session: &Session,
    args: &SendArgs,
    data: &[u8],
    out: &mut Output<'_>,
) -> Result<(), Failure> {
    let s = out.strings();
    let port = &args.port.port;
    let timeout = args.timeout.unwrap_or(TIMEOUT);
    let start = session.end();
    let Some(pattern) = &args.expect else {
        session.write(data).map_err(|e| lost(&e, port, s))?;
        if args.print {
            std::thread::sleep(timeout);
            print_received(session, start, None, out);
        }
        return Ok(());
    };
    let found = session
        .send_and_expect(data, pattern, timeout, MATCH_WINDOW)
        .map_err(|e| lost(&e, port, s));
    let until = match &found {
        Ok(Some(m)) => Some(m.next),
        _ => None,
    };
    if args.print {
        print_received(session, start, until, out);
    }
    match found? {
        Some(_) => Ok(()),
        None => Err(Failure::new(
            Exit::Timeout,
            format!("{} ({}): {pattern}", s.no_match, human(timeout)),
        )),
    }
}

/// Writes the received bytes after `from` (up to `until`, else all) to
/// standard output.
fn print_received(session: &Session, from: Cursor, until: Option<Cursor>, out: &mut Output<'_>) {
    let mut cursor = from;
    while let Ok(chunk) = session.read(cursor, READ_STEP) {
        if chunk.entries.is_empty() {
            break;
        }
        for entry in &chunk.entries {
            if until.is_some_and(|u| entry.seq >= u.0) {
                return;
            }
            if let EntryKind::Data {
                direction: Direction::Rx,
                bytes,
            } = &entry.kind
            {
                out.bytes(bytes);
            }
        }
        cursor = chunk.next;
    }
}

fn human(d: Duration) -> String {
    if d.subsec_millis() == 0 {
        format!("{}s", d.as_secs())
    } else {
        format!("{}ms", d.as_millis())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::exit::Exit;
    use crate::testing::Fixture;

    /// Answers `reply` to the first data the device receives.
    fn answer(
        f: &Fixture,
        port: &'static str,
        reply: &'static [u8],
    ) -> std::thread::JoinHandle<Vec<u8>> {
        let devices = std::sync::Arc::clone(&f.devices);
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                let device = devices.lock().unwrap().get(port).cloned();
                if let Some(device) = device {
                    let got = device.take_written();
                    if !got.is_empty() {
                        device.send(reply);
                        return got;
                    }
                }
                assert!(std::time::Instant::now() < deadline, "nothing was sent");
                std::thread::sleep(Duration::from_millis(1));
            }
        })
    }

    #[test]
    fn sends_text_with_a_line_ending_and_closes_the_port() {
        let f = Fixture::new();
        let ran = f.run(&["send", "mock0", "AT"]);
        assert_eq!((ran.code, ran.out.as_str(), ran.err.as_str()), (0, "", ""));
        assert_eq!(f.devices.lock().unwrap()["mock0"].take_written(), b"AT\r\n");
        assert!(f.manager.list().is_empty(), "the port is closed again");
        f.run(&["send", "mock0", "x", "--ending", "none"]);
        assert_eq!(f.devices.lock().unwrap()["mock0"].take_written(), b"x");
    }

    #[test]
    fn sends_hex_files_and_checksums() {
        let f = Fixture::new();
        let ran = f.run(&[
            "send",
            "mock0",
            "--hex",
            "01 03 00 00 00 01",
            "--checksum",
            "crc16-modbus",
        ]);
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert_eq!(
            f.devices.lock().unwrap()["mock0"].take_written(),
            [0x01, 0x03, 0x00, 0x00, 0x00, 0x01, 0x84, 0x0a],
            "hex gets no line ending"
        );
        f.run(&[
            "send",
            "mock0",
            "AB",
            "--checksum",
            "sum8",
            "--ending",
            "lf",
        ]);
        assert_eq!(
            f.devices.lock().unwrap()["mock0"].take_written(),
            b"AB\x83\n"
        );
        let dir = std::env::temp_dir().join(format!("serialbot-cli-send-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("fw.bin");
        std::fs::write(&file, [0u8, 1, 2]).unwrap();
        let ran = f.run(&["send", "mock0", "--file", file.to_str().unwrap()]);
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert_eq!(f.devices.lock().unwrap()["mock0"].take_written(), [0, 1, 2]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn encodes_text_as_asked() {
        let f = Fixture::new();
        let ran = f.run(&[
            "send",
            "mock0",
            "中",
            "--encoding",
            "gbk",
            "--ending",
            "none",
        ]);
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert_eq!(
            f.devices.lock().unwrap()["mock0"].take_written(),
            [0xd6, 0xd0]
        );
    }

    #[test]
    fn bad_data_is_a_usage_error_before_opening() {
        let f = Fixture::new();
        for args in [
            &["send", "mock0", "--hex", "0g"][..],
            &["send", "mock0", "x", "--encoding", "nope"],
            &["send", "mock0", "--file", "/no/such/file"],
            &["send", "mock0", "", "--ending", "none"],
        ] {
            let ran = f.run(args);
            assert_eq!(ran.code, Exit::Usage as u8, "{args:?}: {}", ran.err);
            assert!(ran.err.starts_with("error: "), "{}", ran.err);
        }
        assert!(f.devices.lock().unwrap().is_empty(), "no port was opened");
    }

    #[test]
    fn ports_that_cannot_be_opened_exit_with_3() {
        let f = Fixture::new();
        let ran = f.run(&["send", "/dev/nothing", "AT"]);
        assert_eq!(ran.code, Exit::Busy as u8);
        assert!(ran.err.contains("cannot open /dev/nothing"), "{}", ran.err);
        let ran = f.run(&["send", "mock0", "AT", "--baud", "0"]);
        assert_eq!(ran.code, Exit::Usage as u8, "{}", ran.err);
    }

    #[test]
    fn waits_for_the_reply_and_prints_it() {
        let f = Fixture::new();
        let device = answer(&f, "mock0", b"OK\r\nmore");
        let ran = f.run(&["send", "mock0", "AT", "--expect", "OK", "--print"]);
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert_eq!(device.join().unwrap(), b"AT\r\n");
        assert_eq!(ran.out, "OK\r\nmore", "the entry holding the match");
    }

    #[test]
    fn no_match_in_time_exits_with_1() {
        let f = Fixture::new();
        let device = answer(&f, "mock0", b"ERROR\r\n");
        let ran = f.run(&[
            "send",
            "mock0",
            "AT",
            "--expect",
            "^OK",
            "--timeout",
            "200ms",
            "--print",
        ]);
        device.join().unwrap();
        assert_eq!(ran.code, Exit::Timeout as u8);
        assert_eq!(ran.out, "ERROR\r\n", "what came instead is printed");
        assert!(
            ran.err
                .contains("no reply matched within the timeout (200ms): ^OK"),
            "{}",
            ran.err
        );
        let ran = f.run(&["send", "mock0", "AT", "--expect", "x", "--timeout", "1s"]);
        assert!(ran.err.contains("(1s)"), "{}", ran.err);
    }

    #[test]
    fn print_without_expect_shows_what_arrives_within_the_timeout() {
        let f = Fixture::new();
        let device = answer(&f, "mock0", b"hello");
        let ran = f.run(&["send", "mock0", "hi", "--print", "--timeout", "300ms"]);
        device.join().unwrap();
        assert_eq!((ran.code, ran.out.as_str()), (0, "hello"));
    }

    #[test]
    fn a_device_that_goes_away_exits_with_4() {
        let f = Fixture::new();
        let devices = std::sync::Arc::clone(&f.devices);
        let unplug = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(d) = devices.lock().unwrap().get("mock0")
                    && !d.take_written().is_empty()
                {
                    d.disconnect();
                    return;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        let ran = f.run(&["send", "mock0", "AT", "--expect", "OK"]);
        unplug.join().unwrap();
        assert_eq!(ran.code, Exit::Disconnected as u8, "{}", ran.err);
        assert!(
            ran.err.contains("the device went away (mock0)"),
            "{}",
            ran.err
        );
    }

    /// A bootloader on `port`: starts with `C` once the port is open, ACKs
    /// every block and the EOT; returns the blocks' bytes.
    fn bootloader(f: &Fixture, port: &'static str) -> std::thread::JoinHandle<Vec<u8>> {
        let devices = std::sync::Arc::clone(&f.devices);
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let device = loop {
                if let Some(d) = devices.lock().unwrap().get(port).cloned() {
                    break d;
                }
                assert!(std::time::Instant::now() < deadline, "never opened");
                std::thread::sleep(Duration::from_millis(1));
            };
            device.send(b"C");
            let mut got = Vec::new();
            while std::time::Instant::now() < deadline {
                let bytes = device.take_written();
                if bytes.is_empty() {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
                got.extend_from_slice(&bytes);
                device.send(&[0x06]);
                if bytes == [0x04] {
                    break;
                }
            }
            got
        })
    }

    #[test]
    fn sends_a_file_to_a_bootloader_with_xmodem() {
        let f = Fixture::new();
        let dir = std::env::temp_dir().join(format!("serialbot-cli-xmodem-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fw.bin");
        std::fs::write(&path, [0xAB; 200]).unwrap();
        let device = bootloader(&f, "mock0");
        let path = path.to_str().unwrap();
        let ran = f.run(&["send", "mock0", "--file", path, "--protocol", "xmodem-1k"]);
        assert_eq!(ran.code, 0, "{ran:?}");
        assert!(
            ran.err.contains("XMODEM-1K: waiting for the device"),
            "{ran:?}"
        );
        assert!(
            ran.err.contains("sent fw.bin (200 B, XMODEM-1K)"),
            "{ran:?}"
        );
        // 200 bytes: one 1 KiB block (more than 128 left), then EOT.
        assert_eq!(device.join().unwrap().len(), 1029 + 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_bootloader_that_never_starts_exits_with_1() {
        let f = Fixture::new();
        let dir =
            std::env::temp_dir().join(format!("serialbot-cli-nostart-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fw.bin");
        std::fs::write(&path, b"x").unwrap();
        let path = path.to_str().unwrap();
        let ran = f.run(&[
            "send",
            "mock0",
            "--file",
            path,
            "--protocol",
            "ymodem",
            "--timeout",
            "100ms",
        ]);
        assert_eq!(ran.code, Exit::Timeout as u8, "{ran:?}");
        assert!(
            ran.err.contains("YMODEM: the receiver did not start"),
            "{ran:?}"
        );
        let ran = f.run(&["send", "mock0", "--protocol", "ymodem", "AT"]);
        assert_eq!(
            ran.code,
            Exit::Usage as u8,
            "--protocol needs --file: {ran:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
