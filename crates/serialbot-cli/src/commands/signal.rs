//! `signal`: set DTR/RTS, send a BREAK or run a reset sequence, then show
//! the line levels.
//!
//! Steps run in order within one open: `dtr=0|1`, `rts=0|1`, a pause such
//! as `100ms`, and `break` or `break=250ms`. The levels last only while the
//! port is open (many systems drop DTR on close), so a reset is written as
//! one sequence, e.g. `dtr=0 rts=1 100ms rts=0`.

use std::time::Duration;

use serialbot_core::SessionManager;
use serialbot_core::session::Session;

use crate::Failure;
use crate::args::SignalArgs;
use crate::commands::send::{lost, open};
use crate::exit::Exit;
use crate::output::{Output, Style};
use crate::values::{parse_duration, parse_level};

/// A break without a length lasts this long.
const DEFAULT_BREAK: Duration = Duration::from_millis(250);

/// One step of a sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Set DTR.
    Dtr(bool),
    /// Set RTS.
    Rts(bool),
    /// Wait.
    Pause(Duration),
    /// Send a BREAK of this length.
    Break(Duration),
}

/// Parses whitespace-separated steps (`dtr=0 rts=1 100ms rts=0 break`).
pub fn parse_steps(text: &str) -> Result<Vec<Step>, String> {
    text.split_whitespace().map(parse_step).collect()
}

fn parse_step(token: &str) -> Result<Step, String> {
    let lower = token.to_ascii_lowercase();
    match lower.split_once('=') {
        Some(("dtr", level)) => parse_level(level).map(Step::Dtr),
        Some(("rts", level)) => parse_level(level).map(Step::Rts),
        Some(("break", length)) => parse_duration(length).map(Step::Break),
        None if lower == "break" => Ok(Step::Break(DEFAULT_BREAK)),
        None => parse_duration(token).map(Step::Pause).map_err(|_| {
            format!(
                "{token:?} is not a step: use dtr=0|1, rts=0|1, break[=250ms] or a pause like 100ms"
            )
        }),
        Some(_) => Err(format!("{token:?} is not a step: use dtr=, rts= or break=")),
    }
}

/// Runs the steps, then prints the levels. Exit codes: 0 done; 2 a bad
/// step; 3 the port cannot be opened, or has no modem lines (a PTY, many
/// network and virtual ports); 4 the device went away.
pub fn run(
    args: &SignalArgs,
    manager: &SessionManager,
    out: &mut Output<'_>,
) -> Result<(), Failure> {
    let mut steps = Vec::new();
    for text in args.steps.iter().chain(args.sequence.iter()) {
        steps.extend(parse_steps(text).map_err(|e| Failure::new(Exit::Usage, e))?);
    }
    if let Some(length) = args.r#break {
        steps.push(Step::Break(length));
    }
    let s = out.strings();
    let session = open(manager, &args.port, s)?;
    let result = apply(&session, &steps, &args.port.port, out);
    let _ = manager.remove(session.id());
    result
}

fn apply(
    session: &Session,
    steps: &[Step],
    port: &str,
    out: &mut Output<'_>,
) -> Result<(), Failure> {
    let s = out.strings();
    let fail = |e: serialbot_core::session::SessionError| match e {
        serialbot_core::session::SessionError::Transport(t) => {
            Failure::new(Exit::Busy, format!("{port}: {t}"))
        }
        other => lost(&other, port, s),
    };
    for step in steps {
        match *step {
            Step::Dtr(level) => session.set_dtr(level).map_err(fail)?,
            Step::Rts(level) => session.set_rts(level).map_err(fail)?,
            Step::Pause(length) => std::thread::sleep(length),
            Step::Break(length) => session.send_break(length).map_err(fail)?,
        }
    }
    let level = |v: Option<bool>| match v {
        Some(true) => "1",
        Some(false) => "0",
        None => "-",
    };
    let outputs = session.output_signals();
    let mut line = format!("DTR {}  RTS {}", level(outputs.dtr), level(outputs.rts));
    if let Ok(inputs) = session.line_states() {
        let input = format!(
            "  CTS {}  DSR {}  RI {}  CD {}",
            u8::from(inputs.cts),
            u8::from(inputs.dsr),
            u8::from(inputs.ri),
            u8::from(inputs.cd)
        );
        line.push_str(&out.paint(Style::Dim, &input));
    }
    out.line(&line);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use serialbot_core::transport::LineStates;

    use super::*;
    use crate::testing::Fixture;

    #[test]
    fn steps_parse_in_order() {
        assert_eq!(
            parse_steps("dtr=0 RTS=on 100ms rts=0 break break=50ms").unwrap(),
            [
                Step::Dtr(false),
                Step::Rts(true),
                Step::Pause(Duration::from_millis(100)),
                Step::Rts(false),
                Step::Break(DEFAULT_BREAK),
                Step::Break(Duration::from_millis(50)),
            ]
        );
        assert!(parse_steps("dtr=2").is_err());
        assert!(parse_steps("100").unwrap_err().contains("not a step"));
        assert!(parse_steps("cts=1").unwrap_err().contains("dtr=, rts="));
        assert!(parse_steps("").unwrap().is_empty());
    }

    #[test]
    fn runs_a_reset_sequence_and_shows_the_levels() {
        let f = Fixture::new();
        let started = Instant::now();
        let ran = f.run(&[
            "signal",
            "mock0",
            "dtr=1",
            "--sequence",
            "rts=1 50ms rts=0",
            "--break",
            "10ms",
        ]);
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert!(
            started.elapsed() >= Duration::from_millis(50),
            "the pause is kept"
        );
        let device = f.devices.lock().unwrap()["mock0"].clone();
        assert!(device.dtr());
        assert!(!device.rts());
        assert_eq!(device.breaks(), [Duration::from_millis(10)]);
        assert_eq!(ran.out, "DTR 1  RTS 0  CTS 0  DSR 0  RI 0  CD 0\n");
        assert!(f.manager.list().is_empty());
    }

    #[test]
    fn without_steps_it_only_shows_the_levels() {
        let f = Fixture::new();
        let ran = f.run(&["--color", "always", "signal", "mock0"]);
        assert_eq!(ran.code, 0, "{}", ran.err);
        assert!(
            ran.out.starts_with("DTR -  RTS -\x1b[2m  CTS 0"),
            "{:?}",
            ran.out
        );
        let _ = LineStates::default();
    }

    #[test]
    fn bad_steps_and_missing_lines_are_reported() {
        let f = Fixture::new();
        let ran = f.run(&["signal", "mock0", "dtr=maybe"]);
        assert_eq!(ran.code, Exit::Usage as u8);
        assert!(f.devices.lock().unwrap().is_empty(), "nothing was opened");
        let ran = f.run(&["signal", "/dev/none", "dtr=1"]);
        assert_eq!(ran.code, Exit::Busy as u8);
    }
}
