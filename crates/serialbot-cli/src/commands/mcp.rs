// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! `mcp serve` and `mcp stdio`: SerialBot's MCP server without the app.
//!
//! `mcp serve` runs the same curated tools as the app on this process's
//! [`SessionManager`]. With `--approve ask` (the default) every change an
//! agent wants is asked in the terminal (`y` allow, `s` allow for this
//! session, `n` deny); `all` and `deny` answer without asking and need no
//! terminal. Every action is logged on standard error.
//!
//! `mcp stdio` is for clients that start MCP servers as child processes
//! (Claude Desktop): it forwards stdin/stdout to a running `mcp serve`
//! (token and port from `cli.json`), or else to the app's server.

use std::io::{self, BufRead, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::Duration;

use serialbot_core::SessionManager;
use serialbot_mcp::approval::{ApprovalRequest, Approvals, Decision};
use serialbot_mcp::clients::{Client, Dirs, Server};
use serialbot_mcp::guard::{Action, AllowAll, Approval, Guard};
use serialbot_mcp::server::{Listen, PORT_ATTEMPTS, RunningServer};
use serialbot_mcp::{DEFAULT_PORT, MCP_PATH, SerialTools, generate_token};

use crate::args::{Approve, ConfigArgs, ServeArgs};
use crate::config::{self, CliConfig};
use crate::exit::Exit;
use crate::output::{Output, Style};
use crate::{Failure, PortLister};

/// How long a request waits for an answer in the terminal.
const ASK_TIMEOUT: Duration = Duration::from_secs(120);

/// Something for the terminal to show.
#[derive(Debug)]
pub enum Event {
    /// The queue of requests waiting for the user changed.
    Queue(Vec<ApprovalRequest>),
    /// An action was decided: allowed, or refused with the reason.
    Done(Action, Result<(), String>),
    /// A line typed in the terminal.
    Line(String),
}

/// Where the server's events go.
pub type Events = Arc<Mutex<mpsc::Sender<Event>>>;

/// Sends `event`, if anyone still listens.
pub fn send(events: &Events, event: Event) {
    let _ = events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .send(event);
}

/// Refuses every action.
struct DenyAll;

impl Guard for DenyAll {
    fn approve(&self, action: Action) -> Approval<'_> {
        Box::pin(std::future::ready(Err(format!(
            "{} is not allowed: the server was started with --approve deny",
            action.tool
        ))))
    }
}

/// Asks `inner`, then reports the decision to the terminal.
struct Logged {
    inner: Arc<dyn Guard>,
    events: Events,
}

impl Guard for Logged {
    fn approve(&self, action: Action) -> Approval<'_> {
        Box::pin(async move {
            let result = self.inner.approve(action.clone()).await;
            send(&self.events, Event::Done(action, result.clone()));
            result
        })
    }
}

/// What `mcp serve` needs from the outside.
pub struct Serve {
    /// The sessions agents work on.
    pub manager: Arc<SessionManager>,
    /// Lists the ports.
    pub list_ports: Arc<PortLister>,
    /// Set by Ctrl+C / SIGTERM.
    pub interrupted: Arc<AtomicBool>,
    /// Where answers are typed.
    pub stdin: Box<dyn Read + Send>,
    /// Whether there is a terminal to ask in.
    pub interactive: bool,
    /// Where `cli.json` lives (`None`: nothing is saved).
    pub config_dir: Option<std::path::PathBuf>,
}

impl std::fmt::Debug for Serve {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Serve")
            .field("interactive", &self.interactive)
            .field("config_dir", &self.config_dir)
            .finish_non_exhaustive()
    }
}

/// How to start a server.
pub struct Start {
    /// The sessions agents work on.
    pub manager: Arc<SessionManager>,
    /// Lists the ports.
    pub list_ports: Arc<PortLister>,
    /// How agents' changes are decided.
    pub approve: Approve,
    /// Who can reach it.
    pub listen: Listen,
    /// Preferred port (the next free of [`PORT_ATTEMPTS`] is used).
    pub port: u16,
    /// Bearer token clients send.
    pub token: String,
    /// The user's protocol files, for `io.read`'s `decode`.
    pub protocols_dir: Option<std::path::PathBuf>,
}

/// A running server and what it reports.
pub struct Running {
    /// The server.
    pub server: RunningServer,
    /// Its token.
    pub token: String,
    /// Requests waiting for the user (`--approve ask` only).
    pub approvals: Option<Arc<Approvals>>,
    /// Queue changes and decisions.
    pub events: mpsc::Receiver<Event>,
    /// For more events (typed lines).
    pub sender: Events,
    runtime: tokio::runtime::Runtime,
}

impl std::fmt::Debug for Running {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Running")
            .field("url", &self.server.url())
            .finish_non_exhaustive()
    }
}

impl Running {
    /// Stops the server.
    pub fn stop(self) {
        let Self {
            server, runtime, ..
        } = self;
        runtime.block_on(server.stop());
    }
}

/// Starts the curated tools on `start.manager`. Exit code 3 when no port
/// is free (or the runtime cannot start).
pub fn start(start: Start) -> Result<Running, Failure> {
    let Start {
        manager,
        list_ports,
        approve,
        listen,
        port,
        token,
        protocols_dir,
    } = start;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| Failure::new(Exit::Busy, e.to_string()))?;
    let (tx, rx) = mpsc::channel();
    let events: Events = Arc::new(Mutex::new(tx));
    let approvals = (approve == Approve::Ask).then(|| {
        let queue_events = Arc::clone(&events);
        Arc::new(Approvals::new(
            ASK_TIMEOUT,
            Box::new(move |queue| send(&queue_events, Event::Queue(queue))),
        ))
    });
    let inner: Arc<dyn Guard> = match (&approvals, approve) {
        (Some(a), _) => Arc::clone(a) as Arc<dyn Guard>,
        (None, Approve::Deny) => Arc::new(DenyAll),
        (None, _) => Arc::new(AllowAll),
    };
    let guard = Arc::new(Logged {
        inner,
        events: Arc::clone(&events),
    });
    let mut tools = SerialTools::new(manager, list_ports).with_guard(guard);
    if let Some(dir) = protocols_dir {
        tools = tools.with_protocols_dir(dir);
    }
    let server = runtime
        .block_on(RunningServer::start(tools, token.clone(), listen, port))
        .map_err(|e| Failure::new(Exit::Busy, e))?;
    Ok(Running {
        server,
        token,
        approvals,
        events: rx,
        sender: events,
        runtime,
    })
}

/// Runs the server until Ctrl+C. Exit codes: 0 after Ctrl+C; 2 `ask`
/// without a terminal, or an invalid token or config file; 3 no port free.
pub fn serve(args: &ServeArgs, env: Serve, out: &mut Output<'_>) -> Result<(), Failure> {
    let m = out.strings().mcp;
    if args.approve == Approve::Ask && !env.interactive {
        return Err(Failure::new(Exit::Usage, m.ask_needs_terminal));
    }
    let token = token(args.token.as_deref(), args.port, env.config_dir.as_deref())?;
    let listen = match args.listen {
        crate::args::ListenArg::Local => Listen::Local,
        crate::args::ListenArg::Lan => Listen::Lan,
    };
    let running = start(Start {
        manager: Arc::clone(&env.manager),
        list_ports: Arc::clone(&env.list_ports),
        approve: args.approve,
        listen,
        port: args.port,
        token,
        protocols_dir: env.config_dir.as_ref().map(|d| d.join("protocols")),
    })?;
    let (server, token) = (&running.server, &running.token);
    out.line(&format!("{}: {}", m.serving, server.url()));
    if let Some(lan) = server.lan_url() {
        out.line(&format!("{}: {lan}", m.lan));
    }
    out.line(&format!("token: {token}"));
    out.line(&out.paint(
        Style::Dim,
        &format!(
            "Claude Code: claude mcp add --transport http serialbot {} --header \"Authorization: Bearer {token}\"",
            server.url()
        ),
    ));
    out.line(&out.paint(Style::Dim, m.stdio_hint));
    if listen == Listen::Lan && args.approve == Approve::All {
        out.error(m.lan_all_warning);
    }
    read_lines(env.stdin, Arc::clone(&running.sender));
    prompt_loop(
        &running.events,
        running.approvals.as_deref(),
        &env.interrupted,
        out,
    );
    running.stop();
    env.manager.close_all();
    Ok(())
}

/// `explicit` (`--token`), else the one in `cli.json`, else a new one
/// saved there; `port` is saved as where to find the server.
pub fn token(explicit: Option<&str>, port: u16, dir: Option<&Path>) -> Result<String, Failure> {
    let usage = |e: String| Failure::new(Exit::Usage, e);
    if let Some(token) = explicit {
        if token.len() < serialbot_mcp::MIN_TOKEN_LEN {
            return Err(usage(format!(
                "the token must be at least {} characters",
                serialbot_mcp::MIN_TOKEN_LEN
            )));
        }
        return Ok(token.to_owned());
    }
    let mut config = match dir {
        Some(dir) => CliConfig::load(dir).map_err(usage)?,
        None => CliConfig::default(),
    };
    if let Some(token) = config.mcp_token.clone() {
        config.mcp_port = Some(port);
        if let Some(dir) = dir {
            config.save(dir).map_err(usage)?;
        }
        return Ok(token);
    }
    let token = generate_token().map_err(|e| usage(format!("cannot generate a token: {e}")))?;
    config.mcp_token = Some(token.clone());
    config.mcp_port = Some(port);
    if let Some(dir) = dir {
        config.save(dir).map_err(usage)?;
    }
    Ok(token)
}

/// `mcp config`: prints the settings that connect an MCP client, for the
/// user to copy into the file their client uses; nothing is written but
/// `cli.json` when it has no token yet. Claude Desktop gets this program
/// with `mcp stdio`; the others get `mcp serve`'s address (the port in
/// `cli.json`, else the default) and token. The snippet goes to standard
/// output (so it can be redirected); the default file (when `dirs` is
/// known) and how to add the entry go to standard error.
pub fn config(
    args: &ConfigArgs,
    dirs: Option<&Dirs>,
    config_dir: Option<&Path>,
    out: &mut Output<'_>,
) -> Result<(), Failure> {
    let m = out.strings().mcp;
    let client = Client::from(args.client);
    let port = config_dir
        .and_then(|dir| CliConfig::load(dir).ok())
        .and_then(|c| c.mcp_port)
        .unwrap_or(DEFAULT_PORT);
    let token = token(None, port, config_dir)?;
    let exe = std::env::current_exe()
        .map_or_else(|_| "serialbot-cli".to_owned(), |p| p.display().to_string());
    let server = Server {
        url: format!("http://127.0.0.1:{port}{MCP_PATH}"),
        token,
        stdio: Some((exe, vec!["mcp".to_owned(), "stdio".to_owned()])),
    };
    let snippet = client
        .snippet(&server)
        .map_err(|e| Failure::new(Exit::Usage, e.to_string()))?;
    if let Some(dirs) = dirs {
        out.error_note(&format!(
            "{} {} {}",
            m.default_file,
            client.config_path(dirs).display(),
            m.maybe_elsewhere
        ));
    }
    out.error_note(&m.add_inside.replace("{section}", client.section()));
    if client != Client::ClaudeDesktop {
        out.error_note(m.needs_serve);
    }
    out.line(snippet.trim_end());
    Ok(())
}

/// Reads typed lines on their own thread (reading blocks).
fn read_lines(stdin: Box<dyn Read + Send>, events: Events) {
    std::thread::spawn(move || {
        for line in io::BufReader::new(stdin).lines() {
            let Ok(line) = line else { break };
            send(&events, Event::Line(line));
        }
    });
}

/// Shows decisions and asks about waiting requests until interrupted.
fn prompt_loop(
    rx: &mpsc::Receiver<Event>,
    approvals: Option<&Approvals>,
    interrupted: &AtomicBool,
    out: &mut Output<'_>,
) {
    let m = out.strings().mcp;
    let mut queue: Vec<ApprovalRequest> = Vec::new();
    let mut asked: Option<u64> = None;
    let mut typed: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    while !interrupted.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Event::Queue(q)) => queue = q,
            Ok(Event::Line(line)) => typed.push_back(line),
            Ok(Event::Done(action, result)) => {
                let (mark, why) = match result {
                    Ok(()) => (out.paint(Style::Dim, "✓"), String::new()),
                    Err(e) => (out.paint(Style::Red, "✗"), format!(" — {e}")),
                };
                let detail = if action.detail.is_empty() {
                    String::new()
                } else {
                    format!(" {}", action.detail)
                };
                out.error_note(&format!(
                    "{mark} AI {} {}{detail}{why}",
                    action.tool, action.port
                ));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        let (Some(approvals), Some(head)) = (approvals, queue.first()) else {
            continue;
        };
        if asked != Some(head.id) {
            asked = Some(head.id);
            let detail = if head.detail.is_empty() {
                String::new()
            } else {
                format!(" {}", head.detail)
            };
            out.error_note(&format!(
                "{} {} {}{detail}",
                out.paint(Style::Bold, m.ask),
                head.tool,
                head.port
            ));
            out.error_note(m.choices);
        }
        let Some(answer) = typed.pop_front() else {
            continue;
        };
        let decision = match answer.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => Decision::Once,
            "s" | "session" => Decision::Session,
            "n" | "no" => Decision::Deny,
            _ => {
                out.error_note(m.choices);
                continue;
            }
        };
        let _ = approvals.answer(head.id, decision);
        queue.remove(0);
    }
}

/// Forwards stdin/stdout to a running `mcp serve`, else the app.
/// Ends when the client closes stdin, or on Ctrl+C / SIGTERM.
pub fn stdio(
    config_dir: Option<&Path>,
    interrupted: &AtomicBool,
    out: &mut Output<'_>,
) -> Result<(), Failure> {
    let m = out.strings().mcp;
    let targets = targets(config_dir);
    if targets.is_empty() {
        return Err(Failure::new(Exit::Busy, m.unreachable));
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| Failure::new(Exit::Busy, e.to_string()))?;
    for (urls, token) in &targets {
        let bridge = runtime.block_on(async {
            tokio::select! {
                result = serialbot_mcp::bridge::run_stdio(urls, token) => result,
                () = until(interrupted) => Ok(()),
            }
        });
        match bridge {
            Ok(()) => return Ok(()),
            Err(serialbot_mcp::bridge::BridgeError::Unreachable { .. }) => {}
            Err(e) => return Err(Failure::new(Exit::Disconnected, e.to_string())),
        }
    }
    Err(Failure::new(Exit::Busy, m.unreachable))
}

/// Returns once `flag` is set.
async fn until(flag: &AtomicBool) {
    while !flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Where to look for a server, in order: `mcp serve` (from `cli.json`),
/// then the app (from its `mcp.json`); each with its fallback ports.
pub fn targets(config_dir: Option<&Path>) -> Vec<(Vec<String>, String)> {
    let Some(dir) = config_dir else {
        return Vec::new();
    };
    let urls = |port: u16| -> Vec<String> {
        (0..PORT_ATTEMPTS)
            .filter_map(|i| port.checked_add(i))
            .map(|p| format!("http://127.0.0.1:{p}{MCP_PATH}"))
            .collect()
    };
    let mut targets = Vec::new();
    if let Ok(CliConfig {
        mcp_token: Some(token),
        mcp_port,
    }) = CliConfig::load(dir)
    {
        targets.push((urls(mcp_port.unwrap_or(DEFAULT_PORT)), token));
    }
    if let Some((port, token)) = config::app_mcp(dir) {
        targets.push((urls(port), token));
    }
    targets
}

#[cfg(test)]
mod tests;
