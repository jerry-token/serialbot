// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Scripts for `run` and the interactive `/run`: send, wait for replies,
//! pause, drive the modem lines, in order, stopping at the first failure.
//!
//! A script is plain text, one step per line:
//!
//! ```text
//! # Wi-Fi check
//! timeout 3s          # how long each `expect` waits from now on (default 5s)
//! send AT             # the text plus the line ending (default CRLF)
//! expect ^OK
//! send AT+GMR
//! expect version:(\S+)
//! wait 100ms
//! hex 01 03 00 00 00 01 84 0a
//! dtr 0
//! rts 1
//! break 250ms
//! mark flashed
//! ending lf           # none, cr, lf or crlf for the following sends
//! ```
//!
//! In `send`, `\r`, `\n`, `\t`, `\e`, `\\` and `\xNN` are escapes. An
//! `expect` sees everything received since the previous send, so a reply
//! that arrives before the `expect` line runs is not missed.
//!
//! The desktop app's command groups (exported as JSON) run too: each
//! command is sent with its own line ending, then its delay is waited.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use regex::bytes::Regex;
use serialbot_core::log::Cursor;
use serialbot_core::session::{Session, SessionError};
use serialbot_core::{encoding, hex};

use crate::args::Ending;
use crate::commands::signal::Step as LineStep;
use crate::values::{parse_duration, parse_level};

/// How long an `expect` waits unless `timeout` says otherwise.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// How many trailing received bytes an `expect` pattern may span.
const MATCH_WINDOW: usize = 64 * 1024;

/// One step of a script.
#[derive(Debug, Clone)]
pub enum Step {
    /// Send these bytes (the line ending is already added).
    Send(Vec<u8>),
    /// Wait until received data matches.
    Expect(Regex, Duration),
    /// Pause.
    Wait(Duration),
    /// Set a modem line or send a BREAK.
    Line(LineStep),
    /// Put a marker in the log.
    Mark(String),
}

/// A step with where it came from, for progress and errors.
#[derive(Debug, Clone)]
pub struct Item {
    /// Line number (text) or command number (JSON), from 1.
    pub line: usize,
    /// The source as written, shown in progress.
    pub source: String,
    /// What to do.
    pub step: Step,
}

/// Parses a text script. Sends get `ending` until an `ending` line changes
/// it and are encoded in `encoding`. Errors name the line.
pub fn parse_text(text: &str, ending: Ending, encoding: &str) -> Result<Vec<Item>, String> {
    let mut items = Vec::new();
    let mut ending = ending;
    let mut timeout = DEFAULT_TIMEOUT;
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let source = strip_comment(raw).trim();
        if source.is_empty() {
            continue;
        }
        let (word, rest) = source
            .split_once(char::is_whitespace)
            .unwrap_or((source, ""));
        let rest = rest.trim();
        let fail = |why: String| format!("line {line}: {why}");
        let step = match word.to_ascii_lowercase().as_str() {
            "send" => {
                let text = unescape(rest).map_err(fail)?;
                let mut bytes =
                    encoding::encode_text(&text, encoding).map_err(|e| fail(e.to_string()))?;
                bytes.extend_from_slice(ending.bytes());
                Step::Send(bytes)
            }
            "hex" => Step::Send(hex::parse_hex(rest).map_err(fail)?),
            "expect" => {
                let pattern = Regex::new(rest).map_err(|e| fail(e.to_string()))?;
                Step::Expect(pattern, timeout)
            }
            "wait" => Step::Wait(parse_duration(rest).map_err(fail)?),
            "dtr" => Step::Line(LineStep::Dtr(parse_level(rest).map_err(fail)?)),
            "rts" => Step::Line(LineStep::Rts(parse_level(rest).map_err(fail)?)),
            "break" if rest.is_empty() => Step::Line(LineStep::Break(Duration::from_millis(250))),
            "break" => Step::Line(LineStep::Break(parse_duration(rest).map_err(fail)?)),
            "mark" if !rest.is_empty() => Step::Mark(rest.to_owned()),
            "timeout" => {
                timeout = parse_duration(rest).map_err(fail)?;
                continue;
            }
            "ending" => {
                ending = match rest.to_ascii_lowercase().as_str() {
                    "none" => Ending::None,
                    "cr" => Ending::Cr,
                    "lf" => Ending::Lf,
                    "crlf" => Ending::Crlf,
                    _ => {
                        return Err(fail(format!(
                            "{rest:?} is not an ending: none, cr, lf or crlf"
                        )));
                    }
                };
                continue;
            }
            _ => {
                return Err(fail(format!(
                    "{word:?} is not a step: send, hex, expect, wait, dtr, rts, break, mark, timeout, ending"
                )));
            }
        };
        items.push(Item {
            line,
            source: source.to_owned(),
            step,
        });
    }
    Ok(items)
}

/// A line starting with `#` is a comment, and so is ` # …` at the end of a
/// line, except in `expect` (the pattern runs to the end of the line). In
/// `send`, `\#` is a literal `#`.
fn strip_comment(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return "";
    }
    if trimmed.to_ascii_lowercase().starts_with("expect") {
        return line;
    }
    line.find(" #").map_or(line, |i| &line[..i])
}

/// Resolves `\r \n \t \e \\ \xNN` in `send` text.
fn unescape(text: &str) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('r') => out.push('\r'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('e') => out.push('\u{1b}'),
            Some('\\') => out.push('\\'),
            Some('#') => out.push('#'),
            Some('x') => {
                let digits: String = chars.by_ref().take(2).collect();
                let value = u8::from_str_radix(&digits, 16)
                    .ok()
                    .filter(|_| digits.len() == 2)
                    .ok_or_else(|| format!("\\x{digits} needs two hex digits"))?;
                out.push(char::from(value));
            }
            other => {
                return Err(format!(
                    "unknown escape \\{}",
                    other.map(String::from).unwrap_or_default()
                ));
            }
        }
    }
    Ok(out)
}

/// The steps of a desktop-app command export (a JSON list of groups).
/// With `group`, only that group (by name, case-insensitive); otherwise
/// the only group, or an error naming the groups to choose from.
pub fn parse_groups(json: &str, group: Option<&str>) -> Result<Vec<Item>, String> {
    let groups = load_groups(json)?;
    let names = || {
        groups
            .iter()
            .map(|g| g.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let chosen = match group {
        Some(want) => groups
            .iter()
            .find(|g| g.name.eq_ignore_ascii_case(want))
            .ok_or_else(|| format!("no group {want:?}; groups: {}", names()))?,
        None if groups.len() == 1 => &groups[0],
        None => return Err(format!("choose a group with --group: {}", names())),
    };
    Ok(chosen.items())
}

/// One group of a desktop-app command export.
#[derive(Debug, Clone)]
pub struct Group {
    /// Its name.
    pub name: String,
    /// Its commands, in order.
    pub commands: Vec<Command>,
}

/// One command of a group.
#[derive(Debug, Clone)]
pub struct Command {
    /// Its name, or its text.
    pub label: String,
    /// What it sends (with its line ending).
    pub bytes: Vec<u8>,
    /// The pause after it when the group runs in order.
    pub delay: Duration,
}

impl Group {
    /// The group as script steps: each command, then its delay.
    pub fn items(&self) -> Vec<Item> {
        let mut items = Vec::new();
        for (i, c) in self.commands.iter().enumerate() {
            items.push(Item {
                line: i + 1,
                source: c.label.clone(),
                step: Step::Send(c.bytes.clone()),
            });
            if !c.delay.is_zero() {
                items.push(Item {
                    line: i + 1,
                    source: format!("wait {}ms", c.delay.as_millis()),
                    step: Step::Wait(c.delay),
                });
            }
        }
        items
    }
}

/// Every group of a desktop-app command export.
pub fn load_groups(json: &str) -> Result<Vec<Group>, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("not a command export: {e}"))?;
    let groups = value
        .as_array()
        .ok_or("not a command export: expected a list of groups")?;
    groups.iter().map(group).collect()
}

fn group(g: &serde_json::Value) -> Result<Group, String> {
    let name = g["name"].as_str().unwrap_or_default().to_owned();
    let commands = g["commands"].as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for (i, c) in commands.iter().enumerate() {
        let text = c["text"].as_str().unwrap_or_default();
        let label = c["name"].as_str().unwrap_or(text);
        let bytes = if c["hex"].as_bool() == Some(true) {
            hex::parse_hex(text).map_err(|e| format!("{name}: command {} ({label}): {e}", i + 1))?
        } else {
            let ending = match c["ending"].as_str() {
                Some("cr") => Ending::Cr,
                Some("lf") => Ending::Lf,
                Some("crlf") => Ending::Crlf,
                _ => Ending::None,
            };
            [text.as_bytes(), ending.bytes()].concat()
        };
        out.push(Command {
            label: label.to_owned(),
            bytes,
            delay: Duration::from_millis(c["delayMs"].as_u64().unwrap_or(0)),
        });
    }
    Ok(Group {
        name,
        commands: out,
    })
}

/// Why a script stopped early.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// An `expect` saw no match in time.
    Timeout,
    /// The device went away or a step failed.
    Failed(String),
    /// Stopped from outside (Ctrl+C, quitting).
    Interrupted,
}

/// What happened to one step, for progress output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// The step's line.
    pub line: usize,
    /// The step as written.
    pub source: String,
    /// `None` when it succeeded.
    pub stop: Option<Stop>,
    /// How long it took.
    pub took: Duration,
}

/// Runs `items` on `session` in order; `report` is called after each step.
/// Stops at the first failure and returns it.
pub fn run(
    session: &Session,
    items: &[Item],
    interrupted: &AtomicBool,
    report: &mut dyn FnMut(&Progress),
) -> Result<(), Stop> {
    let mut cursor = session.end();
    for item in items {
        if interrupted.load(Ordering::Relaxed) {
            return Err(Stop::Interrupted);
        }
        let started = Instant::now();
        let result = step(session, &item.step, &mut cursor, interrupted);
        let progress = Progress {
            line: item.line,
            source: item.source.clone(),
            stop: result.clone().err(),
            took: started.elapsed(),
        };
        report(&progress);
        result?;
    }
    Ok(())
}

fn step(
    session: &Session,
    step: &Step,
    cursor: &mut Cursor,
    interrupted: &AtomicBool,
) -> Result<(), Stop> {
    let failed = |e: SessionError| Stop::Failed(e.to_string());
    match step {
        Step::Send(bytes) => {
            *cursor = session.end();
            session.write(bytes).map(drop).map_err(failed)
        }
        Step::Expect(pattern, timeout) => {
            // Wait in slices so an interruption is noticed.
            let deadline = Instant::now() + *timeout;
            loop {
                let left = deadline.saturating_duration_since(Instant::now());
                let slice = left.min(Duration::from_millis(100));
                match session
                    .wait_for(pattern, *cursor, slice, MATCH_WINDOW)
                    .map_err(failed)?
                {
                    Some(m) => {
                        *cursor = m.next;
                        return Ok(());
                    }
                    None if left.is_zero() => return Err(Stop::Timeout),
                    None if interrupted.load(Ordering::Relaxed) => return Err(Stop::Interrupted),
                    None => {}
                }
            }
        }
        Step::Wait(length) => {
            let deadline = Instant::now() + *length;
            while Instant::now() < deadline {
                if interrupted.load(Ordering::Relaxed) {
                    return Err(Stop::Interrupted);
                }
                std::thread::sleep(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(50)),
                );
            }
            Ok(())
        }
        Step::Line(LineStep::Dtr(level)) => session.set_dtr(*level).map_err(failed),
        Step::Line(LineStep::Rts(level)) => session.set_rts(*level).map_err(failed),
        Step::Line(LineStep::Break(length)) => session.send_break(*length).map_err(failed),
        Step::Line(LineStep::Pause(length)) => {
            std::thread::sleep(*length);
            Ok(())
        }
        Step::Mark(label) => {
            session.mark(label);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serialbot_core::transport::mock::{self, MockDevice};
    use serialbot_core::{SessionId, SessionOptions, SystemClock};

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

    #[test]
    fn text_scripts_parse_every_step() {
        let script = "# setup\n\
                      timeout 2s\n\
                      send AT\\x01 # comment\n\
                      expect ^OK #\\d+\n\
                      ending lf\n\
                      send a\\\\b\\#c\n\
                      hex 01 ff\n\
                      wait 10ms\n\
                      dtr 0\n\
                      rts on\n\
                      break\n\
                      break 50ms\n\
                      mark done\n\
                      \n";
        let items = parse_text(script, Ending::Crlf, "utf-8").unwrap();
        assert_eq!(items.len(), 10);
        assert!(matches!(&items[0].step, Step::Send(b) if b == b"AT\x01\r\n"));
        assert_eq!(items[0].line, 3);
        assert_eq!(items[0].source, "send AT\\x01");
        match &items[1].step {
            Step::Expect(re, timeout) => {
                assert_eq!(re.as_str(), "^OK #\\d+", "a # inside a pattern stays");
                assert_eq!(*timeout, Duration::from_secs(2));
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(&items[2].step, Step::Send(b) if b == b"a\\b#c\n"));
        assert!(
            matches!(&items[3].step, Step::Send(b) if b == &[1, 0xff]),
            "hex gets no ending"
        );
        assert!(matches!(items[4].step, Step::Wait(d) if d == Duration::from_millis(10)));
        assert!(matches!(items[5].step, Step::Line(LineStep::Dtr(false))));
        assert!(matches!(items[6].step, Step::Line(LineStep::Rts(true))));
        assert!(
            matches!(items[7].step, Step::Line(LineStep::Break(d)) if d == Duration::from_millis(250))
        );
        assert!(
            matches!(items[8].step, Step::Line(LineStep::Break(d)) if d == Duration::from_millis(50))
        );
        assert!(matches!(&items[9].step, Step::Mark(l) if l == "done"));
        let gbk = parse_text("send 中\nending none\nsend \\e", Ending::None, "gbk").unwrap();
        assert!(matches!(&gbk[0].step, Step::Send(b) if b == &[0xd6, 0xd0]));
        assert!(matches!(&gbk[1].step, Step::Send(b) if b == b"\x1b"));
    }

    #[test]
    fn bad_lines_are_named() {
        for (script, want) in [
            (
                "send ok\nfrobnicate",
                "line 2: \"frobnicate\" is not a step",
            ),
            ("expect (", "line 1:"),
            ("wait 5", "line 1:"),
            ("dtr maybe", "line 1:"),
            ("hex 0g", "line 1:"),
            ("send \\q", "line 1: unknown escape \\q"),
            ("send \\x4", "line 1: \\x4 needs two hex digits"),
            ("ending dos", "line 1: \"dos\" is not an ending"),
            ("timeout soon", "line 1:"),
            ("break never", "line 1:"),
            ("mark", "line 1: \"mark\" is not a step"),
            ("send 中", "line 1:"),
        ] {
            let err = parse_text(
                script,
                Ending::Crlf,
                if script == "send 中" {
                    "windows-1252"
                } else {
                    "utf-8"
                },
            )
            .unwrap_err();
            assert!(err.starts_with(want), "{script:?}: {err}");
        }
    }

    #[test]
    fn app_command_groups_run_too() {
        let json = r#"[
            {"name": "Demo board", "commands": [
                {"name": "Version", "text": "version", "hex": false, "ending": "crlf", "delayMs": 100},
                {"name": "Raw", "text": "01 02", "hex": true, "ending": "crlf", "delayMs": 0},
                {"text": "x", "ending": "lf"}
            ]},
            {"name": "ESP-AT", "commands": [{"name": "AT", "text": "AT", "ending": "cr", "delayMs": 0}]}
        ]"#;
        let items = parse_groups(json, Some("demo BOARD")).unwrap();
        assert_eq!(items.len(), 4, "a send per command, and the delay");
        assert!(matches!(&items[0].step, Step::Send(b) if b == b"version\r\n"));
        assert_eq!(items[0].source, "Version");
        assert!(matches!(items[1].step, Step::Wait(d) if d == Duration::from_millis(100)));
        assert!(matches!(&items[2].step, Step::Send(b) if b == &[1, 2]));
        assert!(matches!(&items[3].step, Step::Send(b) if b == b"x\n"));
        assert!(
            parse_groups(json, None)
                .unwrap_err()
                .contains("Demo board, ESP-AT")
        );
        assert!(
            parse_groups(json, Some("nope"))
                .unwrap_err()
                .contains("no group")
        );
        let one = r#"[{"name": "Only", "commands": [{"text": "hi", "ending": "none"}]}]"#;
        assert!(matches!(&parse_groups(one, None).unwrap()[0].step, Step::Send(b) if b == b"hi"));
        assert!(
            parse_groups("{}", None)
                .unwrap_err()
                .contains("list of groups")
        );
        assert!(
            parse_groups("not json", None)
                .unwrap_err()
                .contains("not a command export")
        );
        let bad = r#"[{"name": "B", "commands": [{"name": "Bad", "text": "zz", "hex": true}]}]"#;
        assert!(
            parse_groups(bad, None)
                .unwrap_err()
                .contains("command 1 (Bad)")
        );
    }

    /// Answers each line the device receives with the next reply.
    fn answer(
        device: MockDevice,
        replies: &'static [&'static [u8]],
    ) -> std::thread::JoinHandle<Vec<u8>> {
        std::thread::spawn(move || {
            let mut all = Vec::new();
            for reply in replies {
                let deadline = Instant::now() + Duration::from_secs(5);
                loop {
                    let got = device.take_written();
                    if !got.is_empty() {
                        all.extend(got);
                        device.send(reply);
                        break;
                    }
                    assert!(Instant::now() < deadline, "nothing sent");
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            all
        })
    }

    #[test]
    fn runs_in_order_and_reports_each_step() {
        let (s, device) = session();
        let device_thread = answer(device.clone(), &[b"OK\r\n", b"version:1.4\r\n"]);
        let items = parse_text("send AT\nexpect OK\nsend AT+GMR\nexpect version:\\S+\nwait 5ms\ndtr 0\nrts 1\nbreak 5ms\nmark done", Ending::Crlf, "utf-8").unwrap();
        let mut seen = Vec::new();
        run(&s, &items, &AtomicBool::new(false), &mut |p| {
            seen.push((p.line, p.stop.clone()));
        })
        .unwrap();
        assert_eq!(device_thread.join().unwrap(), b"AT\r\nAT+GMR\r\n");
        assert_eq!(seen.len(), 9);
        assert!(seen.iter().all(|(_, stop)| stop.is_none()));
        assert!(!device.dtr() && device.rts());
        assert_eq!(device.breaks(), [Duration::from_millis(5)]);
    }

    #[test]
    fn a_reply_before_the_expect_line_is_not_missed() {
        let (s, device) = session();
        let device_thread = answer(device, &[b"OK\r\n"]);
        // The wait lets the reply arrive before `expect` starts.
        let items = parse_text("send AT\nwait 100ms\nexpect OK", Ending::Crlf, "utf-8").unwrap();
        run(&s, &items, &AtomicBool::new(false), &mut |_| {}).unwrap();
        device_thread.join().unwrap();
    }

    #[test]
    fn stops_at_the_first_failure() {
        let (s, device) = session();
        let items = parse_text(
            "timeout 100ms\nsend AT\nexpect OK\nsend never",
            Ending::Crlf,
            "utf-8",
        )
        .unwrap();
        let mut seen = Vec::new();
        let stop = run(&s, &items, &AtomicBool::new(false), &mut |p| {
            seen.push(p.clone());
        })
        .unwrap_err();
        assert_eq!(stop, Stop::Timeout);
        assert_eq!(seen.last().unwrap().line, 3);
        assert!(seen.last().unwrap().took >= Duration::from_millis(100));
        assert_eq!(
            device.take_written(),
            b"AT\r\n",
            "nothing after the failure"
        );

        let interrupted = AtomicBool::new(true);
        assert_eq!(
            run(&s, &items, &interrupted, &mut |_| {}),
            Err(Stop::Interrupted)
        );
        device.disconnect();
        let items = parse_text("send x", Ending::None, "utf-8").unwrap();
        assert!(matches!(
            run(&s, &items, &AtomicBool::new(false), &mut |_| {}),
            Err(Stop::Failed(_))
        ));
    }

    #[test]
    fn interruptions_end_waits_and_expects_early() {
        let (s, _device) = session();
        let interrupted = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&interrupted);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag.store(true, Ordering::Relaxed);
        });
        let started = Instant::now();
        let items = parse_text("wait 10s", Ending::None, "utf-8").unwrap();
        assert_eq!(
            run(&s, &items, &interrupted, &mut |_| {}),
            Err(Stop::Interrupted)
        );
        interrupted.store(false, Ordering::Relaxed);
        let flag = Arc::clone(&interrupted);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag.store(true, Ordering::Relaxed);
        });
        let items = parse_text("timeout 10s\nexpect never", Ending::None, "utf-8").unwrap();
        assert_eq!(
            run(&s, &items, &interrupted, &mut |_| {}),
            Err(Stop::Interrupted)
        );
        assert!(started.elapsed() < Duration::from_secs(5));
        let pause = [Item {
            line: 1,
            source: "pause".into(),
            step: Step::Line(LineStep::Pause(Duration::from_millis(1))),
        }];
        assert_eq!(
            run(&s, &pause, &AtomicBool::new(false), &mut |_| {}),
            Ok(())
        );
    }
}
