//! The interactive terminal's behaviour: keys, received data, local
//! commands and reconnecting, for one session. It writes to any
//! [`Write`], so it is tested with mock devices and a terminal emulator;
//! `commands/open.rs` only adds the real terminal (raw mode, key events).

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use crossterm::event::KeyEvent;
use serialbot_core::log::{Cursor, Direction, EntryKind};
use serialbot_core::session::{Session, SessionState};
use serialbot_core::{SessionManager, encoding};
use serialbot_jobs::export::{Captures, ExportFormat};
use serialbot_jobs::filesend::FileSends;

use super::feed::Feed;
use super::keys::{Action, Keys, Local, Mode, Slash};
use super::screen::{Bottom, Screen};
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

/// Settings of an interactive session.
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

/// One interactive session on the screen.
pub struct Interactive {
    /// Key state and input box.
    pub keys: Keys,
    /// How received data is shown.
    pub feed: Feed,
    screen: Screen,
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
    wrap: bool,
    lost: bool,
    last_retry: Instant,
    script: Option<Running>,
}

impl std::fmt::Debug for Interactive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Interactive")
            .field("settings", &self.settings)
            .finish_non_exhaustive()
    }
}

impl Interactive {
    /// Starts showing `session`, `width` columns wide. `None` if the
    /// encoding is unknown.
    pub fn new(
        session: Arc<Session>,
        manager: Arc<SessionManager>,
        settings: Settings,
        width: u16,
    ) -> Option<Self> {
        let feed = Feed::new(&settings.encoding, settings.utc_offset_min, settings.color)?;
        let mode = if settings.raw { Mode::Raw } else { Mode::Line };
        Some(Self {
            keys: Keys::new(mode, settings.escape),
            feed,
            screen: Screen::new(width),
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
            wrap: true,
            lost: false,
            last_retry: Instant::now(),
            script: None,
        })
    }

    /// Shows the start: the input box, or a note in raw mode.
    pub fn start(&mut self, out: &mut impl Write) -> io::Result<()> {
        if self.keys.mode == Mode::Raw {
            let note = self.feed.note(self.term().raw_on).join("");
            write!(out, "{note}\r\n")?;
            return out.flush();
        }
        self.redraw(out)
    }

    /// Draws the input box and status bar (after a change or a resize).
    pub fn redraw(&mut self, out: &mut impl Write) -> io::Result<()> {
        if self.keys.mode == Mode::Line {
            let bottom = self.bottom();
            self.screen.draw(out, &bottom)?;
        }
        Ok(())
    }

    /// The terminal is now `width` columns wide.
    pub fn resize(&mut self, out: &mut impl Write, width: u16) -> io::Result<()> {
        self.screen.erase(out)?;
        self.screen.resize(width);
        self.redraw(out)
    }

    /// Handles one key.
    pub fn key(&mut self, out: &mut impl Write, key: KeyEvent) -> io::Result<Flow> {
        let actions = self.keys.key(key);
        self.act(out, actions)
    }

    /// Handles pasted text.
    pub fn paste(&mut self, out: &mut impl Write, text: &str) -> io::Result<Flow> {
        let actions = self.keys.paste(text);
        self.act(out, actions)
    }

    /// Shows what arrived, follows file sends and looks for a lost device.
    pub fn tick(&mut self, out: &mut impl Write) -> io::Result<Flow> {
        let mut lines = Vec::new();
        while let Ok(chunk) = self.session.read(self.cursor, READ_STEP) {
            if chunk.entries.is_empty() {
                break;
            }
            for entry in &chunk.entries {
                if let EntryKind::Data { direction, bytes } = &entry.kind {
                    let n = bytes.len() as u64;
                    match direction {
                        Direction::Rx => self.rx += n,
                        Direction::Tx => self.tx += n,
                    }
                    if self.keys.mode == Mode::Raw && *direction == Direction::Rx {
                        out.write_all(bytes)?;
                        continue;
                    }
                }
                if self.keys.mode == Mode::Line {
                    lines.extend(self.feed.entry(entry));
                }
            }
            self.cursor = chunk.next;
        }
        self.follow_file(&mut lines);
        self.follow_script(&mut lines);
        let flow = self.follow_connection(&mut lines);
        if self.keys.mode == Mode::Raw {
            out.flush()?;
            for line in &lines {
                write!(out, "\r\n{line}\r\n")?;
            }
            out.flush()?;
        } else {
            self.print(out, &lines)?;
        }
        Ok(flow)
    }

    /// Erases the input box and status bar, leaving the output (on quit,
    /// or before a shell).
    pub fn leave(&mut self, out: &mut impl Write) -> io::Result<()> {
        self.screen.erase(out)?;
        if let Some(line) = self.feed.pending() {
            write!(out, "{line}\r\n")?;
        }
        out.flush()
    }

    /// Runs a `/` command as if typed (`-C` at start runs `/log <file>`).
    pub fn run_command(&mut self, out: &mut impl Write, slash: Slash) -> io::Result<Flow> {
        self.act(out, vec![Action::Command(slash)])
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

    fn act(&mut self, out: &mut impl Write, actions: Vec<Action>) -> io::Result<Flow> {
        let mut notes = Vec::new();
        let mut flow = Flow::Continue;
        for action in actions {
            match action {
                Action::Send(bytes) => self.send(&bytes, &mut notes),
                Action::SendLine(line) => self.send_line(&line, &mut notes),
                Action::Local(local) => flow = self.local(local, out, &mut notes)?,
                Action::Command(slash) => flow = self.command(slash, out, &mut notes)?,
                Action::Edited => {}
            }
        }
        // Show what was just sent (echo) before the next tick.
        self.tick(out)?;
        self.notes(out, &notes)?;
        if self.keys.mode == Mode::Line {
            self.redraw(out)?;
        }
        Ok(flow)
    }

    fn notes(&mut self, out: &mut impl Write, notes: &[String]) -> io::Result<()> {
        if notes.is_empty() {
            return Ok(());
        }
        let lines: Vec<String> = notes.iter().flat_map(|n| self.feed.note(n)).collect();
        if self.keys.mode == Mode::Raw {
            for line in &lines {
                write!(out, "\r\n{line}\r\n")?;
            }
            return out.flush();
        }
        self.print(out, &lines)
    }

    fn print(&mut self, out: &mut impl Write, lines: &[String]) -> io::Result<()> {
        let bottom = self.bottom();
        if lines.is_empty() && self.feed.pending().is_none() {
            return Ok(());
        }
        let lines: Vec<String> = if self.wrap {
            lines.to_vec()
        } else {
            let width = usize::from(crossterm::terminal::size().map_or(80, |(w, _)| w));
            lines
                .iter()
                .map(|l| super::screen::truncate(l, width))
                .collect()
        };
        self.screen.print(out, &lines, &bottom)
    }

    fn send(&mut self, bytes: &[u8], notes: &mut Vec<String>) {
        if let Err(e) = self.session.write(bytes) {
            notes.push(format!("{}: {e}", self.term().failed));
        }
    }

    fn send_line(&mut self, line: &str, notes: &mut Vec<String>) {
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

    fn local(
        &mut self,
        local: Local,
        out: &mut impl Write,
        notes: &mut Vec<String>,
    ) -> io::Result<Flow> {
        let t = self.term();
        match local {
            Local::Help => notes.extend(t.help.iter().map(|l| (*l).to_owned())),
            Local::Quit { .. } => return Ok(Flow::Quit),
            Local::Clear => self.clear(out)?,
            Local::ToggleEcho => self.feed.display.echo = !self.feed.display.echo,
            Local::Break => self.result(self.session.send_break(PULSE), notes),
            Local::Hangup => {
                let low = self.session.set_dtr(false);
                std::thread::sleep(PULSE);
                self.result(low.and_then(|()| self.session.set_dtr(true)), notes);
            }
            Local::ToggleCapture => self.toggle_capture(None, notes),
            Local::CycleTimestamp => self.feed.display.stamps = self.feed.display.stamps.next(),
            Local::PortSettings => self.prefill("/baud "),
            Local::SendFile | Local::TypeFile => self.prefill("/send-file "),
            Local::RunScript => self.prefill("/run "),
            Local::Config => notes.push(t.later.to_owned()),
            Local::ToggleWrap => self.wrap = !self.wrap,
            Local::ToggleAddLf => self.feed.display.add_lf = !self.feed.display.add_lf,
            // Received line feeds already start a new line here.
            Local::ToggleAddCr => {}
            Local::Scrollback => notes.push(t.scrollback.to_owned()),
            Local::Display => notes.push(self.display_note()),
            Local::Shell => return Ok(Flow::Shell),
            Local::ToggleDtr => {
                let level = !self.session.output_signals().dtr.unwrap_or(true);
                self.result(self.session.set_dtr(level), notes);
            }
            Local::ToggleRts => {
                let level = !self.session.output_signals().rts.unwrap_or(true);
                self.result(self.session.set_rts(level), notes);
            }
            Local::Reset => self.reset(notes),
            Local::Palette => {
                // Only reached in raw mode: back to the input box.
                self.set_mode(Mode::Line, out, notes)?;
                self.prefill("/");
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
        Ok(Flow::Continue)
    }

    fn command(
        &mut self,
        slash: Slash,
        out: &mut impl Write,
        notes: &mut Vec<String>,
    ) -> io::Result<Flow> {
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
            Slash::Echo => return self.local(Local::ToggleEcho, out, notes),
            Slash::Log(path) => self.toggle_capture(path.map(PathBuf::from), notes),
            Slash::SendFile(path) => self.send_file(&path, notes),
            Slash::Run(path) => self.run_script(&path, notes),
            Slash::Stop => {
                self.stop_jobs_but_capture();
            }
            Slash::Raw => self.set_mode(Mode::Raw, out, notes)?,
            Slash::Dtr => return self.local(Local::ToggleDtr, out, notes),
            Slash::Rts => return self.local(Local::ToggleRts, out, notes),
            Slash::Break => return self.local(Local::Break, out, notes),
            Slash::Reset => self.reset(notes),
            Slash::Clear => self.clear(out)?,
            Slash::Help => return self.local(Local::Help, out, notes),
            Slash::Quit => return Ok(Flow::Quit),
            Slash::Invalid(text) => notes.push(format!("/{text}: {}", t.unknown_command)),
        }
        Ok(Flow::Continue)
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

    fn prefill(&mut self, text: &str) {
        self.keys.editor.clear();
        self.keys.editor.insert_str(text);
    }

    fn clear(&mut self, out: &mut impl Write) -> io::Result<()> {
        self.screen.erase(out)?;
        write!(out, "\x1b[2J\x1b[H")?;
        self.redraw(out)
    }

    fn set_mode(
        &mut self,
        mode: Mode,
        out: &mut impl Write,
        notes: &mut Vec<String>,
    ) -> io::Result<()> {
        if self.keys.mode == mode {
            return Ok(());
        }
        let t = self.term();
        if mode == Mode::Raw {
            self.leave(out)?;
            self.keys.mode = Mode::Raw;
            let note = self.feed.note(t.raw_on).join("");
            write!(out, "{note}\r\n")?;
            out.flush()?;
        } else {
            self.keys.mode = Mode::Line;
            write!(out, "\r\n")?;
            notes.push(t.raw_off.to_owned());
        }
        Ok(())
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
        let items = match items {
            Ok(items) => items,
            Err(e) => {
                notes.push(format!("{}: {e}", t.failed));
                return;
            }
        };
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
        notes.push(format!("{} {path}", t.script_start));
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
        let open = self.session.state() == SessionState::Open;
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

    fn display_note(&self) -> String {
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

    fn bottom(&self) -> Bottom {
        let (input, cursor) = self.input_lines();
        Bottom {
            pending: self.feed.pending(),
            input,
            cursor,
            status: self.status(),
            prompt: if self.keys.search.is_some() {
                "(search) ".into()
            } else {
                "› ".into()
            },
        }
    }

    /// The input box lines and the cursor, or the search being typed.
    fn input_lines(&self) -> (Vec<String>, (usize, usize)) {
        if let Some(search) = &self.keys.search {
            let found = search
                .found
                .and_then(|i| self.keys.editor.history().get(i))
                .map(|line| format!(" → {line}"))
                .unwrap_or_default();
            let text = format!("{}{found}", search.needle);
            return (vec![text], (0, search.needle.chars().count()));
        }
        let text = self.keys.editor.text();
        let lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
        let mut remaining = self.keys.editor.cursor();
        let mut cursor = (0, 0);
        for (i, line) in lines.iter().enumerate() {
            let len = line.chars().count();
            if remaining <= len {
                cursor = (i, remaining);
                break;
            }
            remaining -= len + 1;
        }
        (lines, cursor)
    }

    fn status(&self) -> String {
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
        if self.keys.prefix_pending() {
            parts.push(format!(
                "Ctrl+{} …",
                self.settings.escape.to_ascii_uppercase()
            ));
        } else {
            parts.push(t.help_hint.to_owned());
        }
        let text = parts.join("  ");
        if self.settings.color {
            format!("\x1b[2m{text}\x1b[0m")
        } else {
            text
        }
    }
}

/// `serialbot-20261123-153012.log` in the current directory.
fn default_capture_name() -> PathBuf {
    let now = jiff::Zoned::now();
    PathBuf::from(format!("serialbot-{}.log", now.strftime("%Y%m%d-%H%M%S")))
}

/// The port name, its end kept when it is long (`…/by-id/usb-FTDI-if00`).
fn short_port(port: &str) -> String {
    const MAX: usize = 24;
    let count = port.chars().count();
    if count <= MAX {
        return port.to_owned();
    }
    let tail: String = port.chars().skip(count - (MAX - 1)).collect();
    format!("…{tail}")
}

/// `512`, `12.4k`, `3.1M`.
fn human_bytes(n: u64) -> String {
    match n {
        0..1000 => n.to_string(),
        1000..1_000_000 => format!("{}.{}k", n / 1000, n % 1000 / 100),
        _ => format!("{}.{}M", n / 1_000_000, n % 1_000_000 / 100_000),
    }
}

#[cfg(test)]
mod tests;
