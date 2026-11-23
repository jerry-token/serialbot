// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! The interactive terminal (`open`) for one session: a [`Console`] whose
//! lines scroll above an input box and a status bar. It writes to any
//! [`Write`], so it is tested with mock devices and a terminal emulator;
//! `commands/open.rs` only adds the real terminal (raw mode, key events).

use std::io::{self, Write};
use std::sync::Arc;

use crossterm::event::KeyEvent;
use serialbot_core::SessionManager;
use serialbot_core::session::Session;

use super::console::{Console, Effect};
pub use super::console::{Flow, Settings};
use super::keys::{Action, Keys, Mode, Slash};
use super::screen::{Bottom, Screen};

/// One interactive session on the screen.
pub struct Interactive {
    /// Key state and input box.
    pub keys: Keys,
    /// The session's console.
    pub console: Console,
    screen: Screen,
}

impl std::fmt::Debug for Interactive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Interactive")
            .field("console", &self.console)
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
        let mode = if settings.raw { Mode::Raw } else { Mode::Line };
        let escape = settings.escape;
        Some(Self {
            keys: Keys::new(mode, escape),
            console: Console::new(session, manager, settings)?,
            screen: Screen::new(width),
        })
    }

    /// Shows the start: the input box, or a note in raw mode.
    pub fn start(&mut self, out: &mut impl Write) -> io::Result<()> {
        if self.keys.mode == Mode::Raw {
            let note = self.console.feed.note(self.term().raw_on).join("");
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
        let raw = self.keys.mode == Mode::Raw;
        let (tick, flow) = self.console.tick(raw);
        if raw {
            out.write_all(&tick.raw)?;
            out.flush()?;
            for line in &tick.lines {
                write!(out, "\r\n{line}\r\n")?;
            }
            out.flush()?;
        } else {
            self.print(out, &tick.lines)?;
        }
        Ok(flow)
    }

    /// Erases the input box and status bar, leaving the output (on quit,
    /// or before a shell).
    pub fn leave(&mut self, out: &mut impl Write) -> io::Result<()> {
        self.screen.erase(out)?;
        if let Some(line) = self.console.feed.pending() {
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
        self.console.stop_jobs();
    }

    fn act(&mut self, out: &mut impl Write, actions: Vec<Action>) -> io::Result<Flow> {
        let mut notes = Vec::new();
        let mut flow = Flow::Continue;
        for action in actions {
            let effect = match action {
                Action::Send(bytes) => {
                    self.console.send(&bytes, &mut notes);
                    None
                }
                Action::SendLine(line) => {
                    self.console.send_line(&line, &mut notes);
                    None
                }
                Action::Local(local) => self.console.local(local, &mut notes),
                Action::Command(slash) => self.console.command(slash, &mut notes),
                Action::Edited => None,
            };
            if let Some(effect) = effect {
                flow = self.effect(effect, out, &mut notes)?;
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

    fn effect(
        &mut self,
        effect: Effect,
        out: &mut impl Write,
        notes: &mut Vec<String>,
    ) -> io::Result<Flow> {
        match effect {
            Effect::Quit => return Ok(Flow::Quit),
            Effect::Shell => return Ok(Flow::Shell),
            Effect::Clear => self.clear(out)?,
            Effect::Raw => self.set_mode(Mode::Raw, out, notes)?,
            Effect::Palette => {
                // Only reached in raw mode: back to the input box.
                self.set_mode(Mode::Line, out, notes)?;
                self.prefill("/");
            }
            Effect::Prefill(text) => self.prefill(text),
        }
        Ok(Flow::Continue)
    }

    fn notes(&mut self, out: &mut impl Write, notes: &[String]) -> io::Result<()> {
        if notes.is_empty() {
            return Ok(());
        }
        let lines = self.console.notes(notes);
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
        if lines.is_empty() && self.console.feed.pending().is_none() {
            return Ok(());
        }
        let lines: Vec<String> = if self.console.wrap {
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

    fn term(&self) -> &'static crate::i18n::Term {
        self.console.settings().lang.strings().term
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
            let note = self.console.feed.note(t.raw_on).join("");
            write!(out, "{note}\r\n")?;
            out.flush()?;
        } else {
            self.keys.mode = Mode::Line;
            write!(out, "\r\n")?;
            notes.push(t.raw_off.to_owned());
        }
        Ok(())
    }

    fn bottom(&self) -> Bottom {
        let (input, cursor) = input_lines(&self.keys);
        Bottom {
            pending: self.console.feed.pending(),
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

    fn status(&self) -> String {
        let mut parts = self.console.status_parts();
        parts.push(prefix_hint(&self.keys, self.term()));
        let text = parts.join("  ");
        if self.console.settings().color {
            format!("\x1b[2m{text}\x1b[0m")
        } else {
            text
        }
    }
}

/// `Ctrl+A …` while the prefix waits for its letter, else the help hint.
pub fn prefix_hint(keys: &Keys, t: &crate::i18n::Term) -> String {
    if keys.prefix_pending() {
        format!("Ctrl+{} …", keys.escape.to_ascii_uppercase())
    } else {
        t.help_hint.to_owned()
    }
}

/// The input box lines and the cursor (line, column), or the history
/// search being typed.
pub fn input_lines(keys: &Keys) -> (Vec<String>, (usize, usize)) {
    if let Some(search) = &keys.search {
        let found = search
            .found
            .and_then(|i| keys.editor.history().get(i))
            .map(|line| format!(" → {line}"))
            .unwrap_or_default();
        let text = format!("{}{found}", search.needle);
        return (vec![text], (0, search.needle.chars().count()));
    }
    let text = keys.editor.text();
    let lines: Vec<String> = text.split('\n').map(str::to_owned).collect();
    let mut remaining = keys.editor.cursor();
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

#[cfg(test)]
mod tests;
