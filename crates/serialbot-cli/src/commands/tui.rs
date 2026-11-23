// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! `tui`: the full-screen terminal. [`crate::tui::app::Tui`] runs in the
//! terminal's alternate screen with raw mode and mouse reporting on; the
//! terminal is put back on every way out (quit, Ctrl+C from outside, an
//! error, a panic).

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event,
};
use crossterm::execute;
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use serialbot_core::{SessionManager, encoding};

use crate::args::{ParityArg, TuiArgs};
use crate::commands::mcp;
use crate::commands::signal::parse_steps;
use crate::exit::Exit;
use crate::i18n::Lang;
use crate::output::Output;
use crate::term::console::{Flow, Settings};
use crate::tui::ai::Ai;
use crate::tui::app::{Groups, Options, Tui};
use crate::tui::plot::Pattern;
use crate::tui::view;
use crate::{Failure, PortLister};
use serialbot_mcp::server::Listen;

/// How often the screen is updated.
const FRAME: Duration = Duration::from_millis(33);

/// What `tui` needs besides its options.
pub struct Env {
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
    /// Lists the ports (for agents, with `--mcp`).
    pub list_ports: Arc<PortLister>,
    /// Where `cli.json` lives (the MCP token).
    pub config_dir: Option<std::path::PathBuf>,
}

impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Env")
            .field("terminal", &self.terminal)
            .finish_non_exhaustive()
    }
}

/// The state `tui` starts with: the ports of the command line opened.
/// Exit codes: 2 bad options or no terminal; 3 a port cannot be opened.
pub fn prepare(args: &TuiArgs, manager: &Arc<SessionManager>, env: &Env) -> Result<Tui, Failure> {
    let reset = parse_steps(&args.reset_sequence).map_err(|e| Failure::new(Exit::Usage, e))?;
    if encoding::encode_text("", &args.encoding).is_err() {
        return Err(Failure::new(
            Exit::Usage,
            format!("unknown encoding {:?}", args.encoding),
        ));
    }
    let parity = match args.parity {
        ParityArg::None => 'N',
        ParityArg::Odd => 'O',
        ParityArg::Even => 'E',
    };
    let plot = args
        .plot
        .iter()
        .map(|spec| Pattern::parse(spec))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Failure::new(Exit::Usage, e))?;
    let options = Options {
        port: args.port(""),
        console: Settings {
            port: String::new(),
            baud: args.baud,
            frame: format!("{}{parity}{}", args.data, args.stop),
            encoding: args.encoding.clone(),
            ending: args.ending,
            reset,
            reconnect: !args.no_reconnect,
            raw: false,
            escape: args.escape,
            lang: env.lang,
            color: env.color,
            utc_offset_min: env.utc_offset_min,
        },
        hex: args.hex,
        time: args.time,
        plot,
    };
    let mut tui = Tui::new(Arc::clone(manager), options);
    if let Some(file) = &args.groups {
        tui.groups = Some(Groups::load(file).map_err(|e| Failure::new(Exit::Usage, e))?);
    }
    for port in &args.ports {
        if let Err(message) = tui.open(port, None) {
            tui.close_all();
            return Err(Failure::new(Exit::Busy, message));
        }
    }
    if let Some(&first) = tui.shown.first() {
        tui.select(first);
    }
    if args.mcp {
        let started =
            mcp::token(None, args.mcp_port, env.config_dir.as_deref()).and_then(|token| {
                mcp::start(mcp::Start {
                    manager: Arc::clone(manager),
                    list_ports: Arc::clone(&env.list_ports),
                    approve: args.approve,
                    listen: Listen::Local,
                    port: args.mcp_port,
                    token,
                })
            });
        match started {
            Ok(running) => tui.ai = Some(Ai::new(running)),
            Err(e) => {
                tui.close_all();
                return Err(e);
            }
        }
    }
    Ok(tui)
}

/// Runs `tui` until it is quit.
pub fn run(
    args: &TuiArgs,
    manager: &Arc<SessionManager>,
    env: &Env,
    out: &mut Output<'_>,
) -> Result<(), Failure> {
    if !env.terminal {
        return Err(Failure::new(Exit::Usage, out.strings().tui.needs_terminal));
    }
    let mut tui = prepare(args, manager, env)?;
    let result = full_screen(&mut tui, &env.interrupted);
    tui.close_all();
    result.map_err(|e| Failure::new(Exit::Disconnected, e.to_string()))
}

/// Puts the terminal back however the loop ends.
struct Screen;

impl Screen {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let guard = Self;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        )?;
        Ok(guard)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

fn full_screen(tui: &mut Tui, interrupted: &AtomicBool) -> io::Result<()> {
    let screen = Screen::enter()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    loop {
        tui.tick();
        terminal.draw(|frame| view::render(tui, frame))?;
        if interrupted.load(Ordering::Relaxed) {
            break;
        }
        if !event::poll(FRAME)? {
            continue;
        }
        let height = terminal.size()?.height;
        let flow = match event::read()? {
            Event::Key(key) => tui.key(key, height),
            Event::Paste(text) => tui.paste(&text),
            Event::Mouse(mouse) => {
                tui.mouse(mouse);
                Flow::Continue
            }
            _ => Flow::Continue,
        };
        if flow == Flow::Quit {
            break;
        }
    }
    drop(terminal);
    drop(screen);
    Ok(())
}
