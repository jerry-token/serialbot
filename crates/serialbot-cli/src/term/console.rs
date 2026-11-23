// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! One session's console: received data as lines, local commands, jobs
//! (saving, file sends, scripts) and reconnecting. It draws nothing:
//! [`super::app::Interactive`] (`open`) and the full-screen `tui` each put
//! its lines on their own screen.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use serialbot_core::log::{Cursor, Direction, EntryKind};
use serialbot_core::session::{Session, SessionState};
use serialbot_core::{SessionManager, encoding};
use serialbot_jobs::export::{Captures, ExportFormat};
use serialbot_jobs::filesend::FileSends;

use super::feed::Feed;
use super::keys::{Local, Slash};
use crate::args::Ending;
use crate::commands::signal::Step;
use crate::i18n::Lang;
use crate::script::{self, Progress, Stop};

/// What a running script reports.
enum ScriptEvent {
    Step(Progress),
    Done(Result<(), Stop>),
}

/// A script running on its own thread.
struct Running {
    stop: Arc<AtomicBool>,
    events: mpsc::Receiver<ScriptEvent>,
}

/// How often a lost device is looked for.
const RETRY: Duration = Duration::from_secs(1);

/// How long `Ctrl+A H` holds DTR low, and a `Ctrl+A F` break lasts.
const PULSE: Duration = Duration::from_millis(250);

/// Bytes read from the log per tick.
const READ_STEP: usize = 256 * 1024;

/// File sends go in chunks this big (no pause).
const FILE_CHUNK: usize = 1024;

/// What the caller should do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Keep going.
    Continue,
    /// Quit (`Ctrl+A X`/`Q`, `/quit`).
    Quit,
    /// Run a shell, then come back (`Ctrl+A J`).
    Shell,
    /// The device is gone and reconnecting is off.
    Lost,
}

/// Something only the screen can do, asked for by a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Quit.
    Quit,
    /// Run a shell and come back.
    Shell,
    /// Clear the screen.
    Clear,
    /// Switch to raw mode.
    Raw,
    /// Back to the input box with `/` typed (`Ctrl+A /` in raw mode).
    Palette,
    /// Put this text in the input box.
    Prefill(&'static str),
}

/// Settings of a console.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Port name for the status bar.
    pub port: String,
    /// Baud rate.
    pub baud: u32,
    /// Frame format such as `8N1`.
    pub frame: String,
    /// Encoding of typed and received text.
    pub encoding: String,
    /// Appended to sent lines.
    pub ending: Ending,
    /// What `Ctrl+A 3` / `/reset` does.
    pub reset: Vec<Step>,
    /// Wait for a lost device to come back.
    pub reconnect: bool,
    /// Start in raw mode.
    pub raw: bool,
    /// The prefix key's letter.
    pub escape: char,
    /// Language of notes.
    pub lang: Lang,
    /// Colors on.
    pub color: bool,
    /// Local time zone, minutes east of UTC.
    pub utc_offset_min: i32,
}

/// What one tick brought.
#[derive(Debug, Default)]
pub struct Tick {
    /// Received bytes as they are (raw mode only).
    pub raw: Vec<u8>,
    /// Finished lines to show (line mode), and notes.
    pub lines: Vec<String>,
    /// Bytes received this tick.
    pub rx: u64,
    /// What was received, as it came (line mode).
    pub received: Vec<u8>,
}

/// One session's console.
pub struct Console {
    /// How received data is shown.
    pub feed: Feed,
    /// Lines are wrapped (`Ctrl+A W`).
    pub wrap: bool,
    session: Arc<Session>,
    manager: Arc<SessionManager>,
    settings: Settings,
    cursor: Cursor,
    rx: u64,
    tx: u64,
    captures: Captures,
    capture: Option<PathBuf>,
    files: FileSends,
    file_running: bool,
    lost: bool,
    last_retry: Instant,
    script: Option<Running>,
}

impl std::fmt::Debug for Console {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Console")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl Console {
    /// A console on `session`; `None` if the encoding is unknown.
    pub fn new(
        session: Arc<Session>,
        manager: Arc<SessionManager>,
        settings: Settings,
    ) -> Option<Self> {
        let feed = Feed::new(&settings.encoding, settings.utc_offset_min, settings.color)?;
        Some(Self {
            feed,
            wrap: true,
            session,
            manager,
            settings,
            cursor: Cursor(0),
            rx: 0,
            tx: 0,
            captures: Captures::default(),
            capture: None,
            files: FileSends::default(),
            file_running: false,
            lost: false,
            last_retry: Instant::now(),
            script: None,
        })
    }

    /// The session shown.
    pub fn session(&self) -> &Arc<Session> {
        &self.session
    }

    /// The settings it was made with (the baud rate follows `/baud`).
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Whether the device is gone and being waited for.
    pub fn is_lost(&self) -> bool {
        self.lost
    }

    /// What arrived since the last tick, file and script progress, and
    /// the connection's state. With `raw`, received bytes come as they are
    /// and nothing else from the log is shown.
    pub fn tick(&mut self, raw: bool) -> (Tick, Flow) {
        let mut tick = Tick::default();
        while let Ok(chunk) = self.session.read(self.cursor, READ_STEP) {
            if chunk.entries.is_empty() {
                break;
            }
            for entry in &chunk.entries {
                if let EntryKind::Data { direction, bytes } = &entry.kind {
                    let n = bytes.len() as u64;
                    match direction {
                        Direction::Rx => {
                            self.rx += n;
                            tick.rx += n;
                        }
                        Direction::Tx => self.tx += n,
                    }
                    if raw && *direction == Direction::Rx {
                        tick.raw.extend_from_slice(bytes);
                        continue;
                    }
                }
                if !raw {
                    if let EntryKind::Data {
                        direction: Direction::Rx,
                        bytes,
                    } = &entry.kind
                    {
                        tick.received.extend_from_slice(bytes);
                    }
                    tick.lines.extend(self.feed.entry(entry));
                }
            }
            self.cursor = chunk.next;
        }
        self.follow_file(&mut tick.lines);
        self.follow_script(&mut tick.lines);
        let flow = self.follow_connection(&mut tick.lines);
        (tick, flow)
    }

    /// Notes as lines.
    pub fn notes(&mut self, notes: &[String]) -> Vec<String> {
        notes.iter().flat_map(|n| self.feed.note(n)).collect()
    }

    /// Stops background jobs (script, file send, capture) before the port
    /// closes.
    pub fn stop_jobs(&self) {
        if let Some(running) = &self.script {
            running.stop.store(true, Ordering::Relaxed);
        }
        self.files.stop_all();
        self.captures.stop_all();
    }

    /// Sends bytes; a failure becomes a note.
    pub fn send(&mut self, bytes: &[u8], notes: &mut Vec<String>) {
        if let Err(e) = self.session.write(bytes) {
            notes.push(format!("{}: {e}", self.term().failed));
        }
    }

    /// Sends a typed line: encoded, with the line ending.
    pub fn send_line(&mut self, line: &str, notes: &mut Vec<String>) {
        match encoding::encode_text(line, &self.settings.encoding) {
            Ok(mut bytes) => {
                bytes.extend_from_slice(self.settings.ending.bytes());
                self.send(&bytes, notes);
            }
            Err(e) => notes.push(format!("{}: {e}", self.term().failed)),
        }
    }

    fn term(&self) -> &'static crate::i18n::Term {
        self.settings.lang.strings().term
    }

    /// Runs a `Ctrl+A` command; what the screen has to do comes back.
    pub fn local(&mut self, local: Local, notes: &mut Vec<String>) -> Option<Effect> {
        let t = self.term();
        match local {
            Local::Help => notes.extend(t.help.iter().map(|l| (*l).to_owned())),
            Local::Quit { .. } => return Some(Effect::Quit),
            Local::Clear => return Some(Effect::Clear),
            Local::ToggleEcho => self.feed.display.echo = !self.feed.display.echo,
            Local::Break => self.result(self.session.send_break(PULSE), notes),
            Local::Hangup => {
                let low = self.session.set_dtr(false);
                std::thread::sleep(PULSE);
                self.result(low.and_then(|()| self.session.set_dtr(true)), notes);
            }
            Local::ToggleCapture => self.toggle_capture(None, notes),
            Local::CycleTimestamp => self.feed.display.stamps = self.feed.display.stamps.next(),
            Local::PortSettings => return Some(Effect::Prefill("/baud ")),
            Local::SendFile | Local::TypeFile => return Some(Effect::Prefill("/send-file ")),
            Local::RunScript => return Some(Effect::Prefill("/run ")),
            Local::Config => notes.push(t.later.to_owned()),
            Local::ToggleWrap => self.wrap = !self.wrap,
            Local::ToggleAddLf => self.feed.display.add_lf = !self.feed.display.add_lf,
            // Received line feeds already start a new line here.
            Local::ToggleAddCr => {}
            Local::Scrollback => notes.push(t.scrollback.to_owned()),
            Local::Display => notes.push(self.display_note()),
            Local::Shell => return Some(Effect::Shell),
            Local::ToggleDtr => {
                let level = !self.session.output_signals().dtr.unwrap_or(true);
                self.result(self.session.set_dtr(level), notes);
            }
            Local::ToggleRts => {
                let level = !self.session.output_signals().rts.unwrap_or(true);
                self.result(self.session.set_rts(level), notes);
            }
            Local::Reset => self.reset(notes),
            Local::Palette => return Some(Effect::Palette),
            Local::Focus(_) | Local::SplitSide | Local::SplitStack | Local::TogglePlot => {
                notes.push(t.tui_only.to_owned());
            }
            Local::Unsupported(c) => notes.push(format!(
                "Ctrl+A {} {}",
                c.to_ascii_uppercase(),
                t.unsupported
            )),
            Local::Unknown(c) => notes.push(format!(
                "Ctrl+A {} {}",
                c.to_ascii_uppercase(),
                t.unknown_key
            )),
        }
        None
    }

    /// Runs a `/` command; what the screen has to do comes back.
    pub fn command(&mut self, slash: Slash, notes: &mut Vec<String>) -> Option<Effect> {
        let t = self.term();
        match slash {
            Slash::Baud(baud) => match self.manager.set_baud_rate(self.session.id(), baud) {
                Ok(()) => {
                    self.settings.baud = baud;
                    notes.push(format!("{} {baud}", t.baud));
                }
                Err(e) => notes.push(format!("{}: {e}", t.failed)),
            },
            Slash::Hex => self.feed.display.hex = !self.feed.display.hex,
            Slash::Time => self.feed.display.stamps = self.feed.display.stamps.next(),
            Slash::Echo => return self.local(Local::ToggleEcho, notes),
            Slash::Log(path) => self.toggle_capture(path.map(PathBuf::from), notes),
            Slash::SendFile(path) => self.send_file(&path, notes),
            Slash::Run(path) => self.run_script(&path, notes),
            Slash::Stop => self.stop_jobs_but_capture(),
            Slash::Raw => return Some(Effect::Raw),
            Slash::Dtr => return self.local(Local::ToggleDtr, notes),
            Slash::Rts => return self.local(Local::ToggleRts, notes),
            Slash::Break => return self.local(Local::Break, notes),
            Slash::Reset => self.reset(notes),
            Slash::Clear => return Some(Effect::Clear),
            Slash::Help => return self.local(Local::Help, notes),
            Slash::Quit => return Some(Effect::Quit),
            Slash::Open { .. }
            | Slash::Close
            | Slash::Plot(_)
            | Slash::Groups(_)
            | Slash::Group(_)
            | Slash::Cmd(_) => {
                notes.push(t.tui_only.to_owned());
            }
            Slash::Invalid(text) => notes.push(format!("/{text}: {}", t.unknown_command)),
        }
        None
    }

    fn result(
        &self,
        result: Result<(), serialbot_core::session::SessionError>,
        notes: &mut Vec<String>,
    ) {
        if let Err(e) = result {
            notes.push(format!("{}: {e}", self.term().failed));
        }
    }

    fn reset(&mut self, notes: &mut Vec<String>) {
        for step in self.settings.reset.clone() {
            let result = match step {
                Step::Dtr(level) => self.session.set_dtr(level),
                Step::Rts(level) => self.session.set_rts(level),
                Step::Pause(length) => {
                    std::thread::sleep(length);
                    Ok(())
                }
                Step::Break(length) => self.session.send_break(length),
            };
            if result.is_err() {
                self.result(result, notes);
                return;
            }
        }
    }

    fn toggle_capture(&mut self, path: Option<PathBuf>, notes: &mut Vec<String>) {
        let t = self.term();
        if let Some(previous) = self.capture.take() {
            self.captures.stop(1);
            notes.push(format!("{} {}", t.capture_off, previous.display()));
            if path.is_none() {
                return;
            }
        }
        let path = path.unwrap_or_else(default_capture_name);
        match self.captures.start(
            1,
            &self.session,
            &path,
            ExportFormat::Log,
            self.settings.utc_offset_min,
        ) {
            Ok(_) => {
                notes.push(format!("{} {}", t.capture_on, path.display()));
                self.capture = Some(path);
            }
            Err(e) => notes.push(format!("{}: {e}", t.failed)),
        }
    }

    fn send_file(&mut self, path: &str, notes: &mut Vec<String>) {
        let t = self.term();
        match self.files.start(
            1,
            Arc::clone(&self.session),
            std::path::Path::new(path),
            FILE_CHUNK,
            0,
        ) {
            Ok(status) => {
                self.file_running = status.running;
                notes.push(format!("{} {path} ({} B)", t.sending, status.total));
            }
            Err(e) => notes.push(format!("{}: {e}", t.failed)),
        }
    }

    /// `/stop`: ends a script and a file send (saving to a file goes on).
    fn stop_jobs_but_capture(&mut self) {
        if let Some(running) = &self.script {
            running.stop.store(true, Ordering::Relaxed);
        }
        self.files.stop(1);
    }

    fn run_script(&mut self, path: &str, notes: &mut Vec<String>) {
        let t = self.term();
        if self.script.is_some() {
            notes.push(t.script_busy.to_owned());
            return;
        }
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => {
                notes.push(format!("{}: {path}: {e}", t.failed));
                return;
            }
        };
        let items = if std::path::Path::new(path)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        {
            script::parse_groups(&text, None)
        } else {
            script::parse_text(&text, self.settings.ending, &self.settings.encoding)
        };
        match items {
            Ok(items) => self.run_items(items, path, notes),
            Err(e) => notes.push(format!("{}: {e}", t.failed)),
        }
    }

    /// Runs script steps on their own thread (`/run`, a command group);
    /// `name` says which in the notes.
    pub fn run_items(&mut self, items: Vec<script::Item>, name: &str, notes: &mut Vec<String>) {
        let t = self.term();
        if self.script.is_some() {
            notes.push(t.script_busy.to_owned());
            return;
        }
        let (tx, events) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let session = Arc::clone(&self.session);
        let flag = Arc::clone(&stop);
        std::thread::spawn(move || {
            let result = script::run(&session, &items, &flag, &mut |p| {
                let _ = tx.send(ScriptEvent::Step(p.clone()));
            });
            let _ = tx.send(ScriptEvent::Done(result));
        });
        notes.push(format!("{} {name}", t.script_start));
        self.script = Some(Running { stop, events });
    }

    fn follow_script(&mut self, lines: &mut Vec<String>) {
        let Some(running) = &self.script else {
            return;
        };
        let t = self.term();
        let mut done = false;
        let mut notes = Vec::new();
        while let Ok(event) = running.events.try_recv() {
            match event {
                ScriptEvent::Step(p) => {
                    let mark = if p.stop.is_none() { "✓" } else { "✗" };
                    notes.push(format!("{mark} {}  {}", p.line, p.source));
                }
                ScriptEvent::Done(result) => {
                    done = true;
                    notes.push(match result {
                        Ok(()) => t.script_done.to_owned(),
                        Err(Stop::Timeout) => t.script_timeout.to_owned(),
                        Err(Stop::Interrupted) => t.script_stopped.to_owned(),
                        Err(Stop::Failed(why)) => format!("{}: {why}", t.failed),
                    });
                }
            }
        }
        for note in notes {
            lines.extend(self.feed.note(&note));
        }
        if done {
            self.script = None;
        }
    }

    fn follow_file(&mut self, lines: &mut Vec<String>) {
        if !self.file_running {
            return;
        }
        let status = self.files.status(1);
        if status.running {
            return;
        }
        self.file_running = false;
        let t = self.term();
        let name = status.name.unwrap_or_default();
        let note = match status.error {
            Some(e) => format!("{} {name}: {e}", t.failed),
            None => format!("{} {name} ({} B)", t.sent, status.sent),
        };
        lines.extend(self.feed.note(&note));
    }

    fn follow_connection(&mut self, lines: &mut Vec<String>) -> Flow {
        let state = self.session.state();
        if state == SessionState::Closed {
            // Closed on purpose (`/close` in tui): nothing to wait for.
            self.lost = false;
            return Flow::Continue;
        }
        let open = state == SessionState::Open;
        let t = self.term();
        if open {
            if self.lost {
                self.lost = false;
                lines.extend(self.feed.note(t.back));
            }
            return Flow::Continue;
        }
        if !self.settings.reconnect {
            return Flow::Lost;
        }
        if !self.lost {
            self.lost = true;
            let _ = self.manager.set_auto_reconnect(self.session.id(), true);
            lines.extend(self.feed.note(t.lost));
        }
        if self.last_retry.elapsed() >= RETRY {
            self.last_retry = Instant::now();
            self.manager.reconnect_lost();
        }
        Flow::Continue
    }

    /// `Ctrl+A T`: the display settings as a note.
    pub fn display_note(&self) -> String {
        let t = self.term();
        let d = self.feed.display;
        let flag = |on: bool| if on { t.on } else { t.off };
        format!(
            "{}: hex {} · time {:?} · echo {} · add LF {} · wrap {} · {}",
            t.display,
            flag(d.hex),
            d.stamps,
            flag(d.echo),
            flag(d.add_lf),
            flag(self.wrap),
            self.settings.encoding
        )
    }

    /// The status bar's parts: port, speed, counters, lines, jobs.
    pub fn status_parts(&self) -> Vec<String> {
        let t = self.term();
        let level = |v: Option<bool>| match v {
            Some(true) => "●",
            Some(false) => "○",
            None => "·",
        };
        let signals = self.session.output_signals();
        let mut parts = vec![
            short_port(&self.settings.port),
            format!("{} {}", self.settings.baud, self.settings.frame),
            format!("RX {} TX {}", human_bytes(self.rx), human_bytes(self.tx)),
            format!("DTR{} RTS{}", level(signals.dtr), level(signals.rts)),
        ];
        // Sessions an agent opened in `tui --mcp` have no known speed.
        if self.settings.baud == 0 {
            parts.remove(1);
        }
        if self.lost {
            parts.push(t.waiting.to_owned());
        }
        if self.feed.display.hex {
            parts.push(t.hex.to_owned());
        }
        if let Some(path) = &self.capture {
            parts.push(format!("⏺ {}", path.display()));
        }
        if self.script.is_some() {
            parts.push("▶".to_owned());
        }
        if self.file_running {
            let s = self.files.status(1);
            parts.push(format!(
                "↑ {}%",
                (s.sent * 100).checked_div(s.total).unwrap_or(0)
            ));
        }
        parts
    }
}

/// `serialbot-20261123-153012.log` in the current directory.
pub(crate) fn default_capture_name() -> PathBuf {
    let now = jiff::Zoned::now();
    PathBuf::from(format!("serialbot-{}.log", now.strftime("%Y%m%d-%H%M%S")))
}

/// The port name, its end kept when it is long (`…/by-id/usb-FTDI-if00`).
pub(crate) fn short_port(port: &str) -> String {
    const MAX: usize = 24;
    let count = port.chars().count();
    if count <= MAX {
        return port.to_owned();
    }
    let tail: String = port.chars().skip(count - (MAX - 1)).collect();
    format!("…{tail}")
}

/// `512`, `12.4k`, `3.1M`.
pub(crate) fn human_bytes(n: u64) -> String {
    match n {
        0..1000 => n.to_string(),
        1000..1_000_000 => format!("{}.{}k", n / 1000, n % 1000 / 100),
        _ => format!("{}.{}M", n / 1_000_000, n % 1_000_000 / 100_000),
    }
}

#[cfg(test)]
mod tests;
