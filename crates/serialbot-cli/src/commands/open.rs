// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! `open`: the interactive terminal.
//!
//! In a terminal, [`crate::term::app::Interactive`] runs with the terminal
//! in raw mode: received lines scroll above an input box and a status bar.
//! The terminal is restored on every way out: quitting, Ctrl+C from
//! outside (SIGINT/SIGTERM), an error, or a panic.
//!
//! Without a terminal (input or output redirected) it becomes a plain
//! pipe: received data goes to standard output unchanged, and each line of
//! standard input is sent with the line ending, until the input ends and
//! `--duration`-like quiet follows, or Ctrl+C.

use std::io::{self, BufRead, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use crossterm::{execute, terminal};
use serialbot_core::log::{Cursor, Direction, EntryKind};
use serialbot_core::session::{Session, SessionState};
use serialbot_core::{SessionManager, encoding};
use serialbot_jobs::export::{Captures, ExportFormat};

use crate::Failure;
use crate::args::{OpenArgs, ParityArg};
use crate::commands::send::open as open_port;
use crate::commands::signal::parse_steps;
use crate::exit::Exit;
use crate::i18n::Lang;
use crate::output::Output;
use crate::term::app::{Flow, Interactive, Settings};
use crate::term::feed::Stamps;
use crate::term::keys::Slash;

/// How often the screen is updated.
const FRAME: Duration = Duration::from_millis(16);

/// After standard input ends, how long received data is still shown.
const PIPE_LINGER: Duration = Duration::from_millis(500);

/// Where `open` gets its input and terminal.
pub struct Io {
    /// Standard input, read in pipe mode.
    pub stdin: Box<dyn Read + Send>,
    /// Whether both standard input and output are terminals.
    pub terminal: bool,
    /// Set by Ctrl+C / SIGTERM.
    pub interrupted: Arc<AtomicBool>,
    /// Local time zone, minutes east of UTC.
    pub utc_offset_min: i32,
    /// Colors on.
    pub color: bool,
    /// Language.
    pub lang: Lang,
}

impl std::fmt::Debug for Io {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Io")
            .field("terminal", &self.terminal)
            .finish_non_exhaustive()
    }
}

/// Opens the port and runs the terminal (or the pipe). Exit codes: 0 on
/// quit; 2 bad options; 3 the port cannot be opened; 4 the device went
/// away with `--no-reconnect`.
pub fn run(
    args: &OpenArgs,
    manager: &Arc<SessionManager>,
    io: Io,
    out: &mut Output<'_>,
) -> Result<(), Failure> {
    let s = out.strings();
    let reset = parse_steps(&args.reset_sequence).map_err(|e| Failure::new(Exit::Usage, e))?;
    if encoding::encode_text("", &args.encoding).is_err() {
        return Err(Failure::new(
            Exit::Usage,
            format!("unknown encoding {:?}", args.encoding),
        ));
    }
    let session = open_port(manager, &args.port, s)?;
    let result = if io.terminal {
        let settings = Settings {
            port: args.port.port.clone(),
            baud: args.port.baud,
            frame: frame(args),
            encoding: args.encoding.clone(),
            ending: args.ending,
            reset,
            reconnect: !args.no_reconnect,
            raw: args.raw,
            escape: args.escape,
            lang: io.lang,
            color: io.color,
            utc_offset_min: io.utc_offset_min,
        };
        interactive(&session, manager, settings, args, &io)
            .map_err(|e| Failure::new(Exit::Disconnected, e.to_string()))
            .and_then(|flow| match flow {
                Flow::Lost => Err(Failure::new(
                    Exit::Disconnected,
                    format!("{} ({})", s.disconnected, args.port.port),
                )),
                _ => Ok(()),
            })
    } else {
        pipe(&session, args, io, out)
    };
    let _ = manager.remove(session.id());
    result
}

/// `8N1`, `7E2`, …
fn frame(args: &OpenArgs) -> String {
    let parity = match args.port.parity {
        ParityArg::None => 'N',
        ParityArg::Odd => 'O',
        ParityArg::Even => 'E',
    };
    format!("{}{parity}{}", args.port.data, args.port.stop)
}

/// Puts the terminal back however the interactive loop ends.
struct RawMode;

impl RawMode {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnableBracketedPaste)?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        let _ = terminal::disable_raw_mode();
    }
}

fn interactive(
    session: &Arc<Session>,
    manager: &Arc<SessionManager>,
    settings: Settings,
    args: &OpenArgs,
    io: &Io,
) -> io::Result<Flow> {
    let (width, _) = terminal::size().unwrap_or((80, 24));
    let raw_guard = RawMode::enter()?;
    let mut stdout = io::stdout();
    let Some(mut app) = Interactive::new(Arc::clone(session), Arc::clone(manager), settings, width)
    else {
        return Ok(Flow::Quit);
    };
    app.console.feed.display.hex = args.hex;
    app.console.feed.display.stamps = if args.time { Stamps::Time } else { Stamps::Off };
    if let Some(path) = &args.log {
        // Same as typing `/log <file>`.
        let log = Slash::Log(Some(path.display().to_string()));
        app.run_command(&mut stdout, log)?;
    }
    app.start(&mut stdout)?;

    let paused = Arc::new(AtomicBool::new(false));
    let events = spawn_event_reader(Arc::clone(&paused));
    let flow = loop {
        let mut flow = Flow::Continue;
        match events.recv_timeout(FRAME) {
            Ok(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                flow = app.key(&mut stdout, key)?;
            }
            Ok(Event::Paste(text)) => flow = app.paste(&mut stdout, &text)?,
            Ok(Event::Resize(width, _)) => app.resize(&mut stdout, width)?,
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break Flow::Quit,
        }
        if flow == Flow::Continue {
            flow = app.tick(&mut stdout)?;
        }
        if io.interrupted.load(Ordering::Relaxed) {
            break Flow::Quit;
        }
        match flow {
            Flow::Continue => {}
            Flow::Shell => {
                paused.store(true, Ordering::Relaxed);
                app.leave(&mut stdout)?;
                terminal::disable_raw_mode()?;
                run_shell();
                terminal::enable_raw_mode()?;
                paused.store(false, Ordering::Relaxed);
                app.redraw(&mut stdout)?;
            }
            Flow::Quit | Flow::Lost => break flow,
        }
    };
    app.stop_jobs();
    app.leave(&mut stdout)?;
    drop(raw_guard);
    Ok(flow)
}

/// Reads terminal events on their own thread (reading blocks).
fn spawn_event_reader(paused: Arc<AtomicBool>) -> mpsc::Receiver<Event> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        loop {
            if paused.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            match event::poll(Duration::from_millis(50)) {
                Ok(true) => match event::read() {
                    Ok(e) => {
                        if tx.send(e).is_err() {
                            return;
                        }
                    }
                    Err(_) => return,
                },
                Ok(false) => {}
                Err(_) => return,
            }
        }
    });
    rx
}

/// `Ctrl+A J`: the user's shell until it exits.
fn run_shell() {
    let shell = if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    };
    let _ = std::process::Command::new(shell).status();
}

/// Without a terminal: stdin lines go out, received bytes come in.
fn pipe(
    session: &Arc<Session>,
    args: &OpenArgs,
    io: Io,
    out: &mut Output<'_>,
) -> Result<(), Failure> {
    let s = out.strings();
    let captures = Captures::default();
    if let Some(path) = &args.log {
        captures
            .start(1, session, path, ExportFormat::Log, io.utc_offset_min)
            .map_err(|e| Failure::new(Exit::Usage, e))?;
    }
    let ended = Arc::new(AtomicBool::new(false));
    {
        let session = Arc::clone(session);
        let ended = Arc::clone(&ended);
        let ending = args.ending;
        let label = args.encoding.clone();
        let stdin = io.stdin;
        // Not joined: reading standard input may block until the process
        // ends.
        std::thread::spawn(move || {
            for line in io::BufReader::new(stdin).lines() {
                let Ok(line) = line else { break };
                let Ok(mut bytes) = encoding::encode_text(&line, &label) else {
                    continue;
                };
                bytes.extend_from_slice(ending.bytes());
                if session.write(&bytes).is_err() {
                    break;
                }
            }
            ended.store(true, Ordering::Relaxed);
        });
    }
    let result = stream(session, &ended, &io.interrupted, out, args, s);
    captures.stop_all();
    result
}

fn stream(
    session: &Session,
    ended: &AtomicBool,
    interrupted: &AtomicBool,
    out: &mut Output<'_>,
    args: &OpenArgs,
    s: &crate::i18n::Strings,
) -> Result<(), Failure> {
    let mut cursor = Cursor(0);
    let mut linger: Option<std::time::Instant> = None;
    loop {
        let state = session.state();
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
        if state != SessionState::Open {
            return Err(Failure::new(
                Exit::Disconnected,
                format!("{} ({})", s.disconnected, args.port.port),
            ));
        }
        if interrupted.load(Ordering::Relaxed) {
            return Ok(());
        }
        if ended.load(Ordering::Relaxed) {
            let since = *linger.get_or_insert_with(std::time::Instant::now);
            if since.elapsed() >= PIPE_LINGER {
                return Ok(());
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::exit::Exit;
    use crate::testing::Fixture;

    #[test]
    fn without_a_terminal_it_pipes_lines_and_received_data() {
        let mut f = Fixture::new();
        f.stdin = b"AT\nAT+GMR\n".to_vec();
        let devices = std::sync::Arc::clone(&f.devices);
        let device = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut got = Vec::new();
            loop {
                if let Some(d) = devices.lock().unwrap().get("mock0") {
                    got.extend(d.take_written());
                    if got.ends_with(b"AT+GMR\r") {
                        d.send(b"OK\r\n");
                        return got;
                    }
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        let ran = f.run(&["open", "mock0"]);
        assert_eq!(device.join().unwrap(), b"AT\rAT+GMR\r");
        assert_eq!((ran.code, ran.out.as_str()), (0, "OK\r\n"), "{}", ran.err);
        assert!(f.manager.list().is_empty());
    }

    #[test]
    fn minicom_options_and_bad_values() {
        let f = Fixture::new();
        let ran = f.run(&["open", "-D", "mock0", "--ending", "lf", "-b", "9600"]);
        assert_eq!(ran.code, 0, "{}", ran.err);
        let ran = f.run(&["open", "-Dmock0"]);
        assert_eq!(ran.code, 0, "{}", ran.err);
        let ran = f.run(&["open", "mock0", "--reset-sequence", "dtr=7"]);
        assert_eq!(ran.code, Exit::Usage as u8);
        let ran = f.run(&["open", "mock0", "--encoding", "nope"]);
        assert_eq!(ran.code, Exit::Usage as u8);
        let ran = f.run(&["open", "mock0", "--escape", "ctrl-1"]);
        assert_eq!(ran.code, Exit::Usage as u8);
        let ran = f.run(&[
            "open",
            "mock0",
            "--escape",
            "Ctrl-T",
            "-C",
            "/no/such/dir/x.log",
        ]);
        assert_eq!(ran.code, Exit::Usage as u8, "{}", ran.err);
        let ran = f.run(&["open", "/dev/none"]);
        assert_eq!(ran.code, Exit::Busy as u8);
    }

    #[test]
    fn a_lost_device_ends_the_pipe_with_4() {
        let mut f = Fixture::new();
        f.stdin = Vec::new();
        let devices = std::sync::Arc::clone(&f.devices);
        let unplug = std::thread::spawn(move || {
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
        // Slow stdin so the pipe is still running when the device goes.
        f.stdin_delay = Some(Duration::from_secs(2));
        let ran = f.run(&["open", "mock0"]);
        unplug.join().unwrap();
        assert_eq!(ran.code, Exit::Disconnected as u8, "{}", ran.err);
    }

    #[test]
    fn frames_name_the_format() {
        use clap::Parser as _;
        let cli = crate::args::Cli::try_parse_from([
            "x", "open", "p", "--data", "7", "--parity", "even", "--stop", "2",
        ])
        .unwrap();
        let crate::args::Command::Open(open) = cli.command else {
            panic!("not open");
        };
        assert_eq!(super::frame(&open), "7E2");
        let cli = crate::args::Cli::try_parse_from(["x", "open", "p", "--parity", "odd"]).unwrap();
        let crate::args::Command::Open(open) = cli.command else {
            panic!("not open");
        };
        assert_eq!(super::frame(&open), "8O1");
        assert!(
            format!(
                "{:?}",
                super::Io {
                    stdin: Box::new(std::io::empty()),
                    terminal: false,
                    interrupted: std::sync::Arc::default(),
                    utc_offset_min: 0,
                    color: false,
                    lang: crate::i18n::Lang::En,
                }
            )
            .contains("Io")
        );
    }
}
