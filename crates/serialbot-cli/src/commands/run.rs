// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! `run`: a script (see [`crate::script`]) on one open of the port.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serialbot_core::SessionManager;
use serialbot_core::log::{Cursor, Direction, EntryKind};
use serialbot_core::session::Session;

use crate::Failure;
use crate::args::RunArgs;
use crate::commands::send::open;
use crate::exit::Exit;
use crate::output::{Output, Style};
use crate::script::{self, Progress};
use serialbot_jobs::sequence::{self, Options, Reason, SequenceStatus};

/// Runs the script (`--runs` times); each step is reported on standard
/// error (`✓ 3 send AT`, with the run number when there are several runs,
/// then the tally), received data goes to standard output with `--print`.
/// Exit codes: 0 all steps done; 1 an `expect` timed out (in any run); 2
/// the script is invalid (checked before opening); 3 the port cannot be
/// opened; 4 a step failed or the device went away; Ctrl+C stops it with 0
/// unless a run had failed.
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
    let options = Options {
        runs: (args.runs > 0).then_some(args.runs),
        stop_on_fail: !args.keep_going,
        gap: args.gap.unwrap_or_default(),
    };
    let several = options.runs != Some(1);
    let name = args.script.display().to_string();
    let status = Mutex::new(SequenceStatus::default());
    sequence::run_loops(
        &session,
        &name,
        &items,
        &options,
        interrupted,
        &status,
        &mut |run, p: &Progress| {
            if args.print {
                printed = print_received(&session, printed, out);
            }
            let mark = if p.stop.is_none() {
                out.paint(Style::Dim, "✓")
            } else {
                out.paint(Style::Red, "✗")
            };
            let run = if several {
                out.paint(Style::Dim, &format!("#{run:<3} "))
            } else {
                String::new()
            };
            out.error_note(&format!(
                "{mark} {run}{:>3}  {}  {}",
                p.line,
                p.source,
                out.paint(Style::Dim, &format!("({} ms)", p.took.as_millis()))
            ));
        },
    );
    let _ = manager.remove(session.id());
    let st = status
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if several {
        out.error_note(&tally(&st, s));
    }
    if let Some(why) = &st.error {
        return Err(Failure::new(
            Exit::Disconnected,
            format!("{} ({}): {why}", s.disconnected, args.port.port),
        ));
    }
    if st.failed == 0 {
        return Ok(());
    }
    let message = if several {
        s.runs_failed
            .replace("{failed}", &st.failed.to_string())
            .replace("{done}", &st.done.to_string())
    } else {
        s.no_match.to_owned()
    };
    let timed_out = st.failures.iter().all(|f| f.reason == Reason::Timeout);
    let code = if timed_out {
        Exit::Timeout
    } else {
        Exit::Disconnected
    };
    Err(Failure::new(code, message))
}

/// The tally of several runs, and their times when some passed.
fn tally(st: &SequenceStatus, s: &crate::i18n::Strings) -> String {
    let mut text = s
        .runs_tally
        .replace("{done}", &st.done.to_string())
        .replace("{passed}", &st.passed.to_string())
        .replace("{failed}", &st.failed.to_string());
    if let (Some(fastest), Some(slowest)) = (st.fastest_ms, st.slowest_ms) {
        let average = st.passed_ms / u64::from(st.passed.max(1));
        text.push('\n');
        text.push_str(
            &s.runs_times
                .replace("{fastest}", &fastest.to_string())
                .replace("{average}", &average.to_string())
                .replace("{slowest}", &slowest.to_string()),
        );
    }
    text
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

    #[test]
    fn several_runs_number_their_steps_and_end_with_the_tally() {
        let f = Fixture::new();
        let dir = Dir::new("runs");
        let script = dir.file("ping.txt", "send ping\nexpect OK\n");
        let device = ok_device(&f, 3);
        let ran = f.run(&["run", "mock0", &script, "--runs", "3", "--gap", "10ms"]);
        assert_eq!(device.join().unwrap(), b"ping\r\nping\r\nping\r\n");
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert!(ran.err.contains("✓ #3     1  send ping"), "{}", ran.err);
        assert!(
            ran.err.contains("runs: 3, passed: 3, failed: 0"),
            "{}",
            ran.err
        );
        assert!(ran.err.contains("ms on average"), "{}", ran.err);
    }

    #[test]
    fn failed_runs_are_tallied_and_exit_with_1() {
        let f = Fixture::new();
        let dir = Dir::new("runs-fail");
        let script = dir.file("never.txt", "timeout 20ms\nsend ping\nexpect never\n");
        let ran = f.run(&["run", "mock0", &script, "--runs", "3", "--keep-going"]);
        assert_eq!(ran.code, Exit::Timeout as u8, "{}", ran.err);
        assert!(
            ran.err.contains("runs: 3, passed: 0, failed: 3"),
            "{}",
            ran.err
        );
        assert!(ran.err.contains("3 of 3 runs failed"), "{}", ran.err);
        assert!(!ran.err.contains("average"), "{}", ran.err);

        // Without --keep-going the first failed run ends it.
        let ran = f.run(&["run", "mock0", &script, "--runs", "3"]);
        assert!(
            ran.err.contains("runs: 1, passed: 0, failed: 1"),
            "{}",
            ran.err
        );
    }

    #[test]
    fn zero_runs_go_on_until_ctrl_c() {
        let mut f = Fixture::new();
        f.lang = crate::Lang::Zh;
        let dir = Dir::new("runs-forever");
        let script = dir.file("wait.txt", "wait 5ms\n");
        let flag = std::sync::Arc::clone(&f.interrupted);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let ran = f.run(&["run", "mock0", &script, "--runs", "0"]);
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert!(ran.err.contains("失败：0"), "{}", ran.err);
    }
}
