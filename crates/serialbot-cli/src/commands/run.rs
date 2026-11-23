// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! `run`: a script (see [`crate::script`]) on one open of the port.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serialbot_core::SessionManager;
use serialbot_core::log::{Cursor, Direction, EntryKind};
use serialbot_core::session::Session;

use crate::Failure;
use crate::args::RunArgs;
use crate::commands::send::open;
use crate::exit::Exit;
use crate::output::{Output, Style};
use crate::script::{self, Progress, Stop};

/// Runs the script; each step is reported on standard error (`✓ 3 send
/// AT`), received data goes to standard output with `--print`. Exit
/// codes: 0 all steps done; 1 an `expect` timed out; 2 the script is
/// invalid (checked before opening); 3 the port cannot be opened; 4 a
/// step failed or the device went away; Ctrl+C stops it with 0.
pub fn run(
    args: &RunArgs,
    manager: &SessionManager,
    interrupted: &Arc<AtomicBool>,
    out: &mut Output<'_>,
) -> Result<(), Failure> {
    let s = out.strings();
    let text = std::fs::read_to_string(&args.script).map_err(|e| {
        Failure::new(
            Exit::Usage,
            format!("{} {}: {e}", s.cannot_read, args.script.display()),
        )
    })?;
    let is_json = args
        .script
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let items = if is_json {
        script::parse_groups(&text, args.group.as_deref())
    } else {
        script::parse_text(&text, args.ending, &args.encoding)
    }
    .map_err(|e| Failure::new(Exit::Usage, format!("{}: {e}", args.script.display())))?;
    let session = open(manager, &args.port, s)?;
    let mut printed = session.end();
    let result = script::run(&session, &items, interrupted, &mut |p: &Progress| {
        if args.print {
            printed = print_received(&session, printed, out);
        }
        let mark = if p.stop.is_none() {
            out.paint(Style::Dim, "✓")
        } else {
            out.paint(Style::Red, "✗")
        };
        out.error_note(&format!(
            "{mark} {:>3}  {}  {}",
            p.line,
            p.source,
            out.paint(Style::Dim, &format!("({} ms)", p.took.as_millis()))
        ));
    });
    let _ = manager.remove(session.id());
    match result {
        Ok(()) | Err(Stop::Interrupted) => Ok(()),
        Err(Stop::Timeout) => Err(Failure::new(Exit::Timeout, s.no_match.to_owned())),
        Err(Stop::Failed(why)) => Err(Failure::new(
            Exit::Disconnected,
            format!("{} ({}): {why}", s.disconnected, args.port.port),
        )),
    }
}

/// Writes received bytes after `from` and returns the new position.
fn print_received(session: &Session, from: Cursor, out: &mut Output<'_>) -> Cursor {
    let mut cursor = from;
    while let Ok(chunk) = session.read(cursor, 256 * 1024) {
        if chunk.entries.is_empty() {
            break;
        }
        for entry in &chunk.entries {
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
    cursor
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::exit::Exit;
    use crate::testing::Fixture;

    struct Dir(std::path::PathBuf);

    impl Dir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("serialbot-cli-run-{name}-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, name: &str, text: &str) -> String {
            let path = self.0.join(name);
            std::fs::write(&path, text).unwrap();
            path.to_str().unwrap().to_owned()
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Replies `OK` to every line the device receives, `count` times.
    fn ok_device(f: &Fixture, count: usize) -> std::thread::JoinHandle<Vec<u8>> {
        let devices = std::sync::Arc::clone(&f.devices);
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut all = Vec::new();
            let mut answered = 0;
            while answered < count {
                if let Some(d) = devices.lock().unwrap().get("mock0") {
                    let got = d.take_written();
                    if !got.is_empty() {
                        all.extend(got);
                        d.send(b"OK\r\n");
                        answered += 1;
                    }
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "only {answered} answered"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
            all
        })
    }

    #[test]
    fn runs_a_text_script_and_reports_each_step() {
        let f = Fixture::new();
        let dir = Dir::new("text");
        let script = dir.file(
            "check.txt",
            "# check\nsend AT\nexpect OK\nsend AT+GMR\nexpect OK\n",
        );
        let device = ok_device(&f, 2);
        let ran = f.run(&["run", "mock0", &script, "--print"]);
        assert_eq!(device.join().unwrap(), b"AT\r\nAT+GMR\r\n");
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert_eq!(ran.out, "OK\r\nOK\r\n");
        let steps: Vec<&str> = ran.err.lines().collect();
        assert_eq!(steps.len(), 4, "{}", ran.err);
        assert!(steps[0].starts_with("✓   2  send AT  ("), "{}", steps[0]);
        assert!(steps[3].starts_with("✓   5  expect OK"), "{}", steps[3]);
        assert!(f.manager.list().is_empty());
    }

    #[test]
    fn a_missing_reply_exits_with_1_and_names_the_step() {
        let f = Fixture::new();
        let dir = Dir::new("timeout");
        let script = dir.file("t.txt", "timeout 100ms\nsend AT\nexpect OK\nsend later\n");
        let ran = f.run(&["run", "mock0", &script]);
        assert_eq!(ran.code, Exit::Timeout as u8);
        assert!(ran.err.contains("✗   3  expect OK"), "{}", ran.err);
        assert!(!ran.err.contains("later"), "{}", ran.err);
        assert_eq!(f.devices.lock().unwrap()["mock0"].take_written(), b"AT\r\n");
    }

    #[test]
    fn app_exports_run_by_group() {
        let f = Fixture::new();
        let dir = Dir::new("json");
        let export = dir.file(
            "commands.json",
            r#"[{"name": "Board", "commands": [{"name": "Version", "text": "version", "ending": "crlf", "delayMs": 10}]},
               {"name": "Other", "commands": []}]"#,
        );
        let ran = f.run(&["run", "mock0", &export, "--group", "board"]);
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert_eq!(
            f.devices.lock().unwrap()["mock0"].take_written(),
            b"version\r\n"
        );
        assert!(ran.err.contains("Version"), "{}", ran.err);
        let ran = f.run(&["run", "mock0", &export]);
        assert_eq!(ran.code, Exit::Usage as u8);
        assert!(ran.err.contains("--group: Board, Other"), "{}", ran.err);
    }

    #[test]
    fn invalid_scripts_and_lost_devices() {
        let f = Fixture::new();
        let dir = Dir::new("bad");
        let bad = dir.file("bad.txt", "send ok\nfrobnicate\n");
        let ran = f.run(&["run", "mock0", &bad]);
        assert_eq!(ran.code, Exit::Usage as u8);
        assert!(ran.err.contains("line 2"), "{}", ran.err);
        assert!(
            f.devices.lock().unwrap().is_empty(),
            "checked before opening"
        );
        let ran = f.run(&["run", "mock0", "/no/such/script.txt"]);
        assert_eq!(ran.code, Exit::Usage as u8);

        let wait = dir.file("wait.txt", "wait 2s\nsend x\n");
        let devices = std::sync::Arc::clone(&f.devices);
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(d) = devices.lock().unwrap().get("mock0") {
                    d.disconnect();
                    return;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        let ran = f.run(&["run", "mock0", &wait]);
        assert_eq!(ran.code, Exit::Disconnected as u8, "{}", ran.err);
    }

    #[test]
    fn ctrl_c_stops_it_without_an_error() {
        let f = Fixture::new();
        let dir = Dir::new("stop");
        let script = dir.file("long.txt", "wait 10s\n");
        let flag = std::sync::Arc::clone(&f.interrupted);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let ran = f.run(&["run", "mock0", &script]);
        assert_eq!(ran.code, 0, "{}", ran.err);
    }
}
