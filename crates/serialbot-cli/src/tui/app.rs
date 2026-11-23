//! The full-screen terminal's state: sessions, which of them are shown
//! (up to [`MAX_SHOWN`] panes), the focus, and the one input box that
//! sends to the focused session. Every session is a
//! [`crate::term::console::Console`], as in `open`; this adds the layout.
//! Drawing is in `view.rs`; nothing here touches a real terminal, so it is
//! tested with mock devices and ratatui's `TestBackend`.

use std::collections::VecDeque;
use std::sync::Arc;

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use serialbot_core::SessionManager;
use serialbot_core::session::SessionState;

use super::ai::Ai;
use super::plot::{Pattern, Plot};
use crate::args::PortArgs;
use crate::i18n::{Lang, TuiTexts};
use crate::term::console::{Console, Effect, Flow, Settings};
use crate::term::feed::Stamps;
use crate::term::keys::{Action, Keys, Local, Mode, Slash, Towards};
use serialbot_mcp::approval::Decision;

/// At most this many sessions are shown side by side.
pub const MAX_SHOWN: usize = 4;

/// Finished lines kept per session for scrolling back.
pub const KEEP: usize = 10_000;

/// Narrower than this, only the focused session is shown.
pub const WIDE: u16 = 100;

/// Rows scrolled by one turn of the mouse wheel.
const WHEEL: usize = 3;

/// One session and what it showed.
#[derive(Debug)]
pub struct Pane {
    /// The session's console.
    pub console: Console,
    /// Port name, as typed.
    pub name: String,
    /// Finished lines, oldest first, with their color sequences.
    pub lines: VecDeque<String>,
    /// Rows scrolled back from the bottom (0: following new data).
    pub scroll: usize,
    /// Data arrived while it was not focused.
    pub unread: bool,
    /// Numbers taken from what it received.
    pub plot: Plot,
    /// The waveform is shown (`Ctrl+A V`).
    pub plot_shown: bool,
}

impl Pane {
    fn push(&mut self, lines: Vec<String>) {
        for line in lines {
            if self.lines.len() == KEEP {
                self.lines.pop_front();
            }
            self.lines.push_back(line);
        }
    }

    /// `●` open, `◌` waiting for the device, `○` closed.
    pub fn state_mark(&self) -> &'static str {
        if self.console.is_lost() {
            "◌"
        } else if self.console.session().state() == SessionState::Open {
            "●"
        } else {
            "○"
        }
    }
}

/// How `tui` opens ports and shows them.
#[derive(Debug, Clone)]
pub struct Options {
    /// Serial settings for every port (the port name is replaced).
    pub port: PortArgs,
    /// Console settings for every port (the port name is replaced).
    pub console: Settings,
    /// Start with hex display.
    pub hex: bool,
    /// Start with timestamps.
    pub time: bool,
    /// How the waveform finds numbers (empty: as the App does); given
    /// patterns also show it from the start.
    pub plot: Vec<Pattern>,
}

/// Where the last frame put things, for the mouse.
#[derive(Debug, Default, Clone)]
pub struct Regions {
    /// The session list (empty when hidden).
    pub list: Rect,
    /// Each shown pane: the session's index and its area.
    pub panes: Vec<(usize, Rect)>,
}

/// The full-screen terminal.
pub struct Tui {
    /// Key state and the input box.
    pub keys: Keys,
    /// Every session opened, in order.
    pub panes: Vec<Pane>,
    /// Indexes into `panes` of the sessions shown, left to right.
    pub shown: Vec<usize>,
    /// Index into `panes` of the focused session.
    pub focus: usize,
    /// Panes side by side (`Ctrl+A |`) or one above the other (`Ctrl+A -`).
    pub side_by_side: bool,
    /// Notes shown while no session is open.
    pub messages: Vec<String>,
    /// Where the last frame put things.
    pub regions: Regions,
    /// The last frame was wide enough for the list and several panes.
    pub wide: bool,
    /// The AI panel (`--mcp`).
    pub ai: Option<Ai>,
    manager: Arc<SessionManager>,
    options: Options,
}

impl std::fmt::Debug for Tui {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tui")
            .field("panes", &self.panes.len())
            .field("shown", &self.shown)
            .field("focus", &self.focus)
            .finish_non_exhaustive()
    }
}

impl Tui {
    /// No session yet; [`Tui::open`] adds them.
    pub fn new(manager: Arc<SessionManager>, options: Options) -> Self {
        Self {
            keys: Keys::new(Mode::Line, options.console.escape),
            panes: Vec::new(),
            shown: Vec::new(),
            focus: 0,
            side_by_side: true,
            messages: Vec::new(),
            regions: Regions::default(),
            wide: true,
            ai: None,
            manager,
            options,
        }
    }

    /// The language of every text.
    pub fn lang(&self) -> Lang {
        self.options.console.lang
    }

    /// Whether colors are on.
    pub fn color(&self) -> bool {
        self.options.console.color
    }

    /// The `tui` texts.
    pub fn texts(&self) -> &'static TuiTexts {
        self.lang().strings().tui
    }

    /// The focused session, if any.
    pub fn focused(&self) -> Option<&Pane> {
        self.panes.get(self.focus)
    }

    /// Opens `port` (at `baud`, or the starting baud rate), shows it and
    /// focuses it. A port already open here is focused instead. On
    /// failure the reason comes back, ready to show.
    pub fn open(&mut self, port: &str, baud: Option<u32>) -> Result<(), String> {
        if let Some(i) = self
            .panes
            .iter()
            .position(|p| p.name == port && p.console.session().state() == SessionState::Open)
        {
            self.select(i);
            return Ok(());
        }
        let mut args = self.options.port.clone();
        port.clone_into(&mut args.port);
        if let Some(baud) = baud {
            args.baud = baud;
        }
        let strings = self.lang().strings();
        let session =
            crate::commands::send::open(&self.manager, &args, strings).map_err(|f| f.message)?;
        let mut settings = self.options.console.clone();
        port.clone_into(&mut settings.port);
        settings.baud = args.baud;
        self.add(session, settings)?;
        self.select(self.panes.len() - 1);
        Ok(())
    }

    /// A pane for `session`, not focused.
    fn add(
        &mut self,
        session: Arc<serialbot_core::session::Session>,
        settings: Settings,
    ) -> Result<(), String> {
        let port = settings.port.clone();
        let Some(mut console) = Console::new(session, Arc::clone(&self.manager), settings) else {
            return Err(format!(
                "unknown encoding {:?}",
                self.options.console.encoding
            ));
        };
        console.feed.display.hex = self.options.hex;
        console.feed.display.stamps = if self.options.time {
            Stamps::Time
        } else {
            Stamps::Off
        };
        self.panes.push(Pane {
            console,
            name: port,
            lines: VecDeque::new(),
            scroll: 0,
            unread: false,
            plot: Plot::new(self.options.plot.clone()),
            plot_shown: !self.options.plot.is_empty(),
        });
        Ok(())
    }

    /// Sessions an agent opened (`--mcp`) get a pane too, shown while
    /// there is room.
    fn adopt(&mut self) {
        for summary in self.manager.list() {
            if self
                .panes
                .iter()
                .any(|p| p.console.session().id() == summary.id)
            {
                continue;
            }
            let Ok(session) = self.manager.get(summary.id) else {
                continue;
            };
            let mut settings = self.options.console.clone();
            settings.port.clone_from(&summary.info.name);
            settings.baud = 0;
            settings.frame = String::new();
            if self.add(session, settings).is_ok() {
                let i = self.panes.len() - 1;
                self.panes[i].unread = true;
                if self.panes.len() == 1 {
                    self.select(0);
                } else if self.shown.len() < MAX_SHOWN {
                    self.shown.push(i);
                }
            }
        }
    }

    /// Focuses session `i`, showing it in the focused pane's place if it
    /// was not shown (or in a new pane while there is room).
    pub fn select(&mut self, i: usize) {
        if i >= self.panes.len() {
            return;
        }
        if !self.shown.contains(&i) {
            if self.shown.len() < MAX_SHOWN {
                self.shown.push(i);
            } else if let Some(slot) = self.shown.iter().position(|s| *s == self.focus) {
                self.shown[slot] = i;
            }
        }
        self.focus = i;
        self.panes[i].unread = false;
    }

    /// Handles one key.
    pub fn key(&mut self, key: KeyEvent, height: u16) -> Flow {
        if key.kind == KeyEventKind::Release {
            return Flow::Continue;
        }
        if self.answer(key) {
            return Flow::Continue;
        }
        if !self.keys.prefix_pending() && self.keys.search.is_none() {
            let page = usize::from(height / 2).max(1);
            match key.code {
                KeyCode::PageUp => return self.scroll_focused(page, true),
                KeyCode::PageDown => return self.scroll_focused(page, false),
                _ => {}
            }
        }
        let actions = self.keys.key(key);
        self.act(actions)
    }

    /// While the agent waits for an answer, plain keys answer it: `y`
    /// allow, `s` allow for this session, `n` or Esc deny; other plain keys
    /// do nothing. `Ctrl` keys (and `Ctrl+A` commands) work as usual.
    fn answer(&mut self, key: KeyEvent) -> bool {
        let Some(ai) = self.ai.as_mut() else {
            return false;
        };
        let modified = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        if ai.asking().is_none() || modified || self.keys.prefix_pending() {
            return false;
        }
        let decision = match key.code {
            KeyCode::Char('y' | 'Y') => Decision::Once,
            KeyCode::Char('s' | 'S') => Decision::Session,
            KeyCode::Char('n' | 'N') | KeyCode::Esc => Decision::Deny,
            _ => return true,
        };
        ai.answer(decision);
        true
    }

    /// Handles pasted text.
    pub fn paste(&mut self, text: &str) -> Flow {
        let actions = self.keys.paste(text);
        self.act(actions)
    }

    /// Mouse: a click focuses a pane or picks a session from the list,
    /// the wheel scrolls the pane under it.
    pub fn mouse(&mut self, event: MouseEvent) {
        let at = Position::new(event.column, event.row);
        let over = self
            .regions
            .panes
            .iter()
            .find(|(_, area)| area.contains(at))
            .map(|(i, _)| *i);
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(i) = over {
                    self.select(i);
                } else if self.regions.list.contains(at) {
                    // Rows start under the list's top border.
                    let row = usize::from(event.row.saturating_sub(self.regions.list.y + 1));
                    self.select(row);
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                if let Some(pane) = over.and_then(|i| self.panes.get_mut(i)) {
                    if event.kind == MouseEventKind::ScrollUp {
                        pane.scroll += WHEEL;
                    } else {
                        pane.scroll = pane.scroll.saturating_sub(WHEEL);
                    }
                }
            }
            _ => {}
        }
    }

    /// Takes in what every session received and follows their jobs.
    pub fn tick(&mut self) {
        if let Some(ai) = self.ai.as_mut() {
            ai.poll();
            self.adopt();
        }
        let focus = self.focus;
        for (i, pane) in self.panes.iter_mut().enumerate() {
            let (tick, _) = pane.console.tick(false);
            if tick.rx > 0 && i != focus {
                pane.unread = true;
            }
            if !tick.lines.is_empty() && pane.scroll > 0 {
                // Stay on what is being read while new lines come in.
                pane.scroll += tick.lines.len();
            }
            pane.plot.push(&tick.received);
            pane.push(tick.lines);
        }
    }

    /// Stops every session's jobs and closes their ports (on quit).
    pub fn close_all(&mut self) {
        if let Some(ai) = self.ai.as_mut() {
            ai.stop();
        }
        for pane in &self.panes {
            pane.console.stop_jobs();
            let _ = self.manager.remove(pane.console.session().id());
        }
    }

    fn scroll_focused(&mut self, rows: usize, up: bool) -> Flow {
        if let Some(pane) = self.panes.get_mut(self.focus) {
            pane.scroll = if up {
                pane.scroll + rows
            } else {
                pane.scroll.saturating_sub(rows)
            };
        }
        Flow::Continue
    }

    fn act(&mut self, actions: Vec<Action>) -> Flow {
        let mut notes = Vec::new();
        let mut flow = Flow::Continue;
        for action in actions {
            let effect = match action {
                Action::Edited => None,
                Action::Local(local) => self.local(local, &mut notes),
                Action::Command(slash) => self.command(slash, &mut notes),
                Action::Send(bytes) => {
                    self.with_console(&mut notes, |c, n| c.send(&bytes, n));
                    None
                }
                Action::SendLine(line) => {
                    self.with_console(&mut notes, |c, n| c.send_line(&line, n));
                    None
                }
            };
            if let Some(effect) = effect {
                flow = self.effect(effect, &mut notes);
            }
        }
        // Show what was just sent (echo) at once.
        self.tick();
        self.note(&notes);
        if let Some(pane) = self.panes.get_mut(self.focus) {
            // Typing follows the newest data again.
            if !notes.is_empty() {
                pane.scroll = 0;
            }
        }
        flow
    }

    /// Runs `f` on the focused console, or notes that none is open.
    fn with_console<R>(
        &mut self,
        notes: &mut Vec<String>,
        f: impl FnOnce(&mut Console, &mut Vec<String>) -> R,
    ) -> Option<R> {
        if let Some(pane) = self.panes.get_mut(self.focus) {
            pane.scroll = 0;
            Some(f(&mut pane.console, notes))
        } else {
            notes.push(self.texts().no_session.to_owned());
            None
        }
    }

    fn local(&mut self, local: Local, notes: &mut Vec<String>) -> Option<Effect> {
        match local {
            Local::Quit { .. } => Some(Effect::Quit),
            Local::Help => {
                self.help(notes);
                None
            }
            Local::Focus(towards) => {
                self.move_focus(towards);
                None
            }
            Local::SplitSide | Local::SplitStack => {
                self.side_by_side = local == Local::SplitSide;
                self.show_another();
                None
            }
            Local::TogglePlot => {
                self.plot(None, notes);
                None
            }
            other => self.with_console(notes, |c, n| c.local(other, n)).flatten(),
        }
    }

    fn command(&mut self, slash: Slash, notes: &mut Vec<String>) -> Option<Effect> {
        match slash {
            Slash::Open { port, baud } => {
                if let Err(e) = self.open(&port, baud) {
                    notes.push(e);
                }
                None
            }
            Slash::Close => {
                if let Some(pane) = self.panes.get(self.focus) {
                    pane.console.stop_jobs();
                    let _ = self.manager.close(pane.console.session().id());
                    notes.push(format!("{} {}", self.texts().closed, pane.name));
                } else {
                    notes.push(self.texts().no_session.to_owned());
                }
                None
            }
            Slash::Plot(arg) => {
                self.plot(arg.as_deref(), notes);
                None
            }
            Slash::Quit => Some(Effect::Quit),
            Slash::Help => {
                self.help(notes);
                None
            }
            other => self
                .with_console(notes, |c, n| c.command(other, n))
                .flatten(),
        }
    }

    fn effect(&mut self, effect: Effect, notes: &mut Vec<String>) -> Flow {
        let t = self.texts();
        match effect {
            Effect::Quit => return Flow::Quit,
            Effect::Shell => notes.push(t.no_shell.to_owned()),
            Effect::Raw => notes.push(t.no_raw.to_owned()),
            Effect::Clear => {
                if let Some(pane) = self.panes.get_mut(self.focus) {
                    pane.lines.clear();
                    pane.scroll = 0;
                }
            }
            Effect::Palette => self.prefill("/"),
            Effect::Prefill(text) => self.prefill(text),
        }
        Flow::Continue
    }

    /// `/plot`: on/off; `auto`; `clear`; `color <curve> <color>`; or a
    /// pattern (which also shows it).
    fn plot(&mut self, arg: Option<&str>, notes: &mut Vec<String>) {
        let t = self.texts();
        let Some(pane) = self.panes.get_mut(self.focus) else {
            notes.push(t.no_session.to_owned());
            return;
        };
        let words: Vec<&str> = arg.unwrap_or_default().split_whitespace().collect();
        match words.as_slice() {
            [] => pane.plot_shown = !pane.plot_shown,
            ["auto"] => {
                pane.plot = Plot::default();
                pane.plot_shown = true;
            }
            ["clear"] => pane.plot.clear(),
            ["color", curve, color] => {
                if let Err(e) = pane.plot.set_color(curve, color) {
                    notes.push(format!("{}: {e}", t.plot_unknown));
                }
            }
            _ => match Pattern::parse(arg.unwrap_or_default()) {
                Ok(pattern) => {
                    pane.plot = Plot::new(vec![pattern]);
                    pane.plot_shown = true;
                }
                Err(e) => notes.push(e),
            },
        }
    }

    fn help(&self, notes: &mut Vec<String>) {
        let term = self.lang().strings().term;
        notes.extend(self.texts().help.iter().map(|l| (*l).to_owned()));
        notes.extend(term.help.iter().map(|l| (*l).to_owned()));
    }

    fn prefill(&mut self, text: &str) {
        self.keys.editor.clear();
        self.keys.editor.insert_str(text);
    }

    /// Notes go to the focused session, or the start screen.
    fn note(&mut self, notes: &[String]) {
        if notes.is_empty() {
            return;
        }
        match self.panes.get_mut(self.focus) {
            Some(pane) => {
                let lines = pane.console.notes(notes);
                pane.push(lines);
            }
            None => self.messages.extend(notes.iter().cloned()),
        }
    }

    /// The previous or next shown pane; on a narrow screen (one pane),
    /// the previous or next session.
    fn move_focus(&mut self, towards: Towards) {
        if self.panes.is_empty() {
            return;
        }
        let order: Vec<usize> = if self.shown.len() > 1 && self.wide {
            self.shown.clone()
        } else {
            (0..self.panes.len()).collect()
        };
        let at = order.iter().position(|i| *i == self.focus).unwrap_or(0);
        let n = order.len();
        let next = match towards {
            Towards::Next => (at + 1) % n,
            Towards::Previous => (at + n - 1) % n,
        };
        self.select(order[next]);
    }

    /// Splitting shows a session not shown yet, if there is one.
    fn show_another(&mut self) {
        if self.shown.len() >= MAX_SHOWN {
            return;
        }
        if let Some(i) = (0..self.panes.len()).find(|i| !self.shown.contains(i)) {
            self.shown.push(i);
        }
    }
}

#[cfg(test)]
mod tests;
