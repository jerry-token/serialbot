//! `serialbot-cli`: SerialBot in the terminal.
//!
//! Another front end next to the desktop app: every device action goes
//! through [`serialbot_core::SessionManager`]. [`run`] takes the arguments
//! and a [`Context`] (where output goes, how ports open), so every command
//! is tested with mock devices and in-memory output; `main.rs` only wires
//! the real terminal and serial ports in.

pub mod args;
pub mod commands;
pub mod exit;
pub mod i18n;
pub mod output;
pub mod term;
pub mod values;

use std::ffi::OsString;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use clap::FromArgMatches as _;
use serialbot_core::SessionManager;
use serialbot_core::transport::TransportError;
use serialbot_core::transport::serial::PortInfo;

use crate::args::{Cli, Command};
use crate::exit::Exit;
use crate::i18n::Lang;
use crate::output::Output;

/// Lists the serial ports present (the real one or a fake in tests).
pub type PortLister = dyn Fn() -> Result<Vec<PortInfo>, TransportError> + Send + Sync;

/// Everything a command needs from the outside world.
pub struct Context<'a> {
    /// Standard output: results, received data.
    pub out: &'a mut dyn Write,
    /// Standard error: progress, warnings, errors.
    pub err: &'a mut dyn Write,
    /// Whether standard output is a terminal (colors, tables).
    pub out_is_terminal: bool,
    /// Whether `NO_COLOR` is set to something non-empty.
    pub no_color: bool,
    /// Language of messages and help.
    pub lang: Lang,
    /// Opens the ports.
    pub manager: Arc<SessionManager>,
    /// Lists the ports.
    pub list_ports: Arc<PortLister>,
    /// Set when the user presses Ctrl+C.
    pub interrupted: Arc<AtomicBool>,
    /// The local time zone, minutes east of UTC, for timestamps.
    pub utc_offset_min: i32,
    /// Standard input (`open` reads it when there is no terminal).
    pub stdin: Option<Box<dyn std::io::Read + Send>>,
    /// Whether standard input and output are both terminals (`open` is
    /// interactive only then).
    pub interactive: bool,
}

impl std::fmt::Debug for Context<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("out_is_terminal", &self.out_is_terminal)
            .field("lang", &self.lang)
            .finish_non_exhaustive()
    }
}

/// Runs the command line `args` (the first item is the program name) and
/// returns the process exit code ([`Exit`]). Help and version go to
/// `ctx.out` with code 0; argument errors to `ctx.err` with code 2.
pub fn run<I, T>(args: I, ctx: &mut Context<'_>) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args = args::minicom_device(args.into_iter().map(Into::into).collect());
    let cli = match args::command(ctx.lang)
        .try_get_matches_from(args)
        .and_then(|m| Cli::from_arg_matches(&m))
    {
        Ok(cli) => cli,
        Err(e) => {
            let code = if e.use_stderr() {
                Exit::Usage
            } else {
                Exit::Ok
            };
            let text = if ctx.out_is_terminal {
                e.render().ansi().to_string()
            } else {
                e.render().to_string()
            };
            let target: &mut dyn Write = if e.use_stderr() { ctx.err } else { ctx.out };
            // Nothing sensible is left to do if the terminal is gone.
            let _ = target.write_all(text.as_bytes());
            return code as u8;
        }
    };
    let color = cli.color.enabled(ctx.out_is_terminal, ctx.no_color);
    let stdin = ctx.stdin.take();
    let mut out = Output::new(
        ctx.out,
        ctx.err,
        ctx.lang,
        cli.color.enabled(ctx.out_is_terminal, ctx.no_color),
    );
    let result = match &cli.command {
        Command::Ports(a) => commands::ports::run(a, &*ctx.list_ports, &mut out),
        Command::Send(a) => commands::send::run(a, &ctx.manager, &mut out),
        Command::Signal(a) => commands::signal::run(a, &ctx.manager, &mut out),
        Command::Open(a) => {
            let io = commands::open::Io {
                stdin: stdin.unwrap_or_else(|| Box::new(std::io::empty())),
                terminal: ctx.interactive,
                interrupted: Arc::clone(&ctx.interrupted),
                utc_offset_min: ctx.utc_offset_min,
                color,
                lang: ctx.lang,
            };
            commands::open::run(a, &ctx.manager, io, &mut out)
        }
        Command::Monitor(a) => commands::monitor::run(
            a,
            &ctx.manager,
            &ctx.interrupted,
            ctx.utc_offset_min,
            &mut out,
        ),
    };
    match result {
        Ok(()) => Exit::Ok as u8,
        Err(failure) => {
            out.error(&failure.message);
            failure.exit as u8
        }
    }
}

/// Why a command failed: the message for the user and the exit code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Exit code to return.
    pub exit: Exit,
    /// What went wrong, in the user's language.
    pub message: String,
}

impl Failure {
    /// A failure with `exit` and `message`.
    pub fn new(exit: Exit, message: impl Into<String>) -> Self {
        Self {
            exit,
            message: message.into(),
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! A context with mock ports and in-memory output.
    #![allow(clippy::unwrap_used, reason = "test helpers")]

    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use serialbot_core::transport::mock::{self, MockDevice};
    use serialbot_core::transport::serial::{PortInfo, PortKind, SerialConfig};
    use serialbot_core::transport::{Connection, TransportError};
    use serialbot_core::{SessionManager, SessionOptions, SystemClock};

    use super::{Context, Lang, run};

    /// Mock devices by port name, as they are opened.
    pub(crate) type Devices = Arc<Mutex<HashMap<String, MockDevice>>>;

    /// What one run printed and returned.
    #[derive(Debug)]
    pub(crate) struct Ran {
        pub(crate) code: u8,
        pub(crate) out: String,
        pub(crate) err: String,
    }

    /// Ports `mock0` (USB) and `mock1` (unknown) are listed; `mock*` paths
    /// open as mock devices, anything else is not found.
    pub(crate) struct Fixture {
        pub(crate) devices: Devices,
        pub(crate) manager: Arc<SessionManager>,
        pub(crate) ports: Vec<PortInfo>,
        pub(crate) lang: Lang,
        pub(crate) terminal: bool,
        pub(crate) interrupted: Arc<AtomicBool>,
        /// What `open` reads as standard input.
        pub(crate) stdin: Vec<u8>,
        /// Standard input ends only after this long (a slow pipe).
        pub(crate) stdin_delay: Option<Duration>,
    }

    /// Standard input that ends after a delay.
    struct Slow(Duration, std::io::Cursor<Vec<u8>>);

    impl std::io::Read for Slow {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.1.read(buf)?;
            if n == 0 {
                std::thread::sleep(self.0);
            }
            Ok(n)
        }
    }

    impl Fixture {
        pub(crate) fn new() -> Self {
            let devices: Devices = Arc::default();
            let registry = Arc::clone(&devices);
            let opener = move |config: &SerialConfig| -> Result<Connection, TransportError> {
                if !config.path.starts_with("mock") {
                    return Err(TransportError::NotFound(config.path.clone()));
                }
                let (connection, device) = mock::connection(&config.path, Duration::from_millis(5));
                registry.lock().unwrap().insert(config.path.clone(), device);
                Ok(connection)
            };
            let manager = Arc::new(SessionManager::new(
                Box::new(opener),
                Arc::new(SystemClock),
                SessionOptions::default(),
            ));
            let ports = vec![
                PortInfo {
                    path: "mock0".into(),
                    kind: PortKind::Usb {
                        vid: 0x1a86,
                        pid: 0x7523,
                        serial_number: Some("A1".into()),
                        manufacturer: Some("wch.cn".into()),
                        product: Some("USB Serial".into()),
                    },
                },
                PortInfo {
                    path: "mock1".into(),
                    kind: PortKind::Unknown,
                },
            ];
            Self {
                devices,
                manager,
                ports,
                lang: Lang::En,
                terminal: false,
                interrupted: Arc::default(),
                stdin: Vec::new(),
                stdin_delay: None,
            }
        }

        /// Runs `serialbot-cli <args>`.
        pub(crate) fn run(&self, args: &[&str]) -> Ran {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let ports = self.ports.clone();
            let mut ctx = Context {
                out: &mut out,
                err: &mut err,
                out_is_terminal: self.terminal,
                no_color: false,
                lang: self.lang,
                manager: Arc::clone(&self.manager),
                list_ports: Arc::new(move || Ok(ports.clone())),
                interrupted: Arc::clone(&self.interrupted),
                utc_offset_min: 0,
                stdin: Some(Box::new(Slow(
                    self.stdin_delay.unwrap_or_default(),
                    std::io::Cursor::new(self.stdin.clone()),
                ))),
                interactive: false,
            };
            let code = run(
                std::iter::once("serialbot-cli").chain(args.iter().copied()),
                &mut ctx,
            );
            assert!(format!("{ctx:?}").contains("Context"));
            Ran {
                code,
                out: String::from_utf8(out).unwrap(),
                err: String::from_utf8(err).unwrap(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Fixture;
    use super::*;

    #[test]
    fn help_and_version_succeed_and_bad_arguments_are_usage_errors() {
        let f = Fixture::new();
        let help = f.run(&["--help"]);
        assert_eq!(help.code, 0);
        assert!(help.out.contains("ports"), "{}", help.out);
        let version = f.run(&["--version"]);
        assert_eq!(version.code, 0);
        assert!(version.out.contains(env!("CARGO_PKG_VERSION")));
        let bad = f.run(&["ports", "--nope"]);
        assert_eq!(bad.code, Exit::Usage as u8);
        assert!(bad.err.contains("--nope"), "{}", bad.err);
        assert!(bad.out.is_empty());
        let none = f.run(&[]);
        assert_eq!(none.code, Exit::Usage as u8);
    }

    #[test]
    fn failures_carry_their_exit_code() {
        let failure = Failure::new(Exit::Busy, "taken");
        assert_eq!(
            (failure.exit, failure.message.as_str()),
            (Exit::Busy, "taken")
        );
    }
}
