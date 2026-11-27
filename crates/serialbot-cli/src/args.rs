// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! The command line: subcommands and their options, as plain data.
//!
//! Help texts come from [`crate::i18n::Strings::help`], keyed by
//! `subcommand.option`, so `--help` follows the system language like every
//! other message; a test checks that every option has a text in both
//! languages.

use std::path::PathBuf;
use std::time::Duration;

use clap::{ArgAction, ArgGroup, Args, CommandFactory, Parser, Subcommand, ValueEnum};
use regex::bytes::Regex;
use serialbot_core::transport::serial::{DataBits, FlowControl, Parity, SerialConfig, StopBits};

use crate::i18n::Lang;
use crate::values::{Checksum, parse_duration, parse_level};

/// `serialbot-cli [--color …] <command> …`
#[derive(Debug, Parser)]
#[command(
    name = "serialbot-cli",
    version,
    long_version = concat!(
        env!("CARGO_PKG_VERSION"),
        "\nCopyright (C) 2026 Jerry <support@aiany.cc>",
        "\nGPL-3.0-only: https://github.com/jerry-token/serialbot"
    ),
    disable_help_flag = true,
    disable_version_flag = true,
    disable_help_subcommand = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// When to use colors.
    #[arg(
        long,
        value_enum,
        default_value_t = ColorChoice::Auto,
        global = true,
        hide_possible_values = true
    )]
    pub color: ColorChoice,
    /// Print help.
    #[arg(short, long, action = ArgAction::Help, global = true)]
    help: Option<bool>,
    /// Print version.
    #[arg(short = 'V', long, action = ArgAction::Version)]
    version: Option<bool>,
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// The subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// List the serial ports.
    Ports(PortsArgs),
    /// Send data once, optionally waiting for a reply.
    Send(SendArgs),
    /// Only receive, streaming to standard output.
    Monitor(MonitorArgs),
    /// Set DTR/RTS, send a BREAK or run a reset sequence.
    Signal(SignalArgs),
    /// The interactive terminal.
    Open(OpenArgs),
    /// The full-screen terminal: several sessions side by side.
    Tui(TuiArgs),
    /// Run a script.
    Run(RunArgs),
    /// MCP server for AI agents.
    Mcp(McpArgs),
    /// Print a shell completion script.
    Completions(CompletionsArgs),
}

/// Options of `completions`.
#[derive(Debug, Args)]
pub struct CompletionsArgs {
    /// The shell.
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}

/// `mcp serve`, `mcp stdio` or `mcp config`.
#[derive(Debug, Args)]
pub struct McpArgs {
    /// Which.
    #[command(subcommand)]
    pub command: McpCommand,
}

/// The `mcp` subcommands.
#[derive(Debug, Subcommand)]
pub enum McpCommand {
    /// Serve MCP over HTTP.
    Serve(ServeArgs),
    /// Bridge stdio to a running server.
    Stdio,
    /// Print the settings that connect an MCP client.
    Config(ConfigArgs),
}

/// Options of `mcp config`.
#[derive(Debug, Args)]
pub struct ConfigArgs {
    /// Which client.
    #[arg(value_enum)]
    pub client: ClientArg,
}

/// The clients `mcp config` knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ClientArg {
    /// Claude Desktop (starts `mcp stdio`).
    ClaudeDesktop,
    /// Cursor (`~/.cursor/mcp.json`).
    Cursor,
    /// VS Code (the user `mcp.json`).
    Vscode,
    /// Codex (`~/.codex/config.toml`).
    Codex,
    /// Gemini CLI (`~/.gemini/settings.json`).
    Gemini,
    /// opencode (`~/.config/opencode/opencode.json`).
    Opencode,
}

impl From<ClientArg> for serialbot_mcp::clients::Client {
    fn from(arg: ClientArg) -> Self {
        match arg {
            ClientArg::ClaudeDesktop => Self::ClaudeDesktop,
            ClientArg::Cursor => Self::Cursor,
            ClientArg::Vscode => Self::VsCode,
            ClientArg::Codex => Self::Codex,
            ClientArg::Gemini => Self::Gemini,
            ClientArg::Opencode => Self::OpenCode,
        }
    }
}

/// Options of `mcp serve`.
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Who can reach the server.
    #[arg(long, value_enum, default_value_t = ListenArg::Local)]
    pub listen: ListenArg,
    /// Preferred port.
    #[arg(long, default_value_t = serialbot_mcp::DEFAULT_PORT)]
    pub port: u16,
    /// Bearer token.
    #[arg(long)]
    pub token: Option<String>,
    /// How agents' changes are approved.
    #[arg(long, value_enum, default_value_t = Approve::Ask)]
    pub approve: Approve,
}

/// `--listen`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ListenArg {
    /// 127.0.0.1 only.
    Local,
    /// Every address.
    Lan,
}

/// `--approve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Approve {
    /// Ask in the terminal.
    Ask,
    /// Allow everything.
    All,
    /// Refuse every change.
    Deny,
}

/// Options of `ports`.
#[derive(Debug, Args)]
pub struct PortsArgs {
    /// One JSON object per line.
    #[arg(long)]
    pub json: bool,
}

/// Which port to open and how; shared by every subcommand that opens one.
#[derive(Debug, Clone, Args)]
pub struct PortArgs {
    /// Serial port or network address.
    pub port: String,
    /// Baud rate.
    #[arg(short, long, default_value_t = 115_200, value_parser = clap::value_parser!(u32).range(1..))]
    pub baud: u32,
    /// Data bits.
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u8).range(5..=8))]
    pub data: u8,
    /// Parity.
    #[arg(long, value_enum, default_value_t = ParityArg::None)]
    pub parity: ParityArg,
    /// Stop bits.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=2))]
    pub stop: u8,
    /// Flow control.
    #[arg(long, value_enum, default_value_t = FlowArg::None)]
    pub flow: FlowArg,
    /// DTR level right after opening.
    #[arg(long, value_parser = parse_level, value_name = "0|1")]
    pub dtr: Option<bool>,
    /// RTS level right after opening.
    #[arg(long, value_parser = parse_level, value_name = "0|1")]
    pub rts: Option<bool>,
}

impl PortArgs {
    /// The settings to open the port with. Network addresses ignore the
    /// serial settings.
    pub fn config(&self) -> SerialConfig {
        let mut config = SerialConfig::new(self.port.clone(), self.baud);
        // Both ranges are checked by the parser.
        config.data_bits = DataBits::try_from(self.data).unwrap_or_default();
        config.stop_bits = StopBits::try_from(self.stop).unwrap_or_default();
        config.parity = match self.parity {
            ParityArg::None => Parity::None,
            ParityArg::Odd => Parity::Odd,
            ParityArg::Even => Parity::Even,
        };
        config.flow_control = match self.flow {
            FlowArg::None => FlowControl::None,
            FlowArg::Software => FlowControl::Software,
            FlowArg::Hardware => FlowControl::Hardware,
        };
        config.dtr_on_open = self.dtr;
        config.rts_on_open = self.rts;
        config
    }
}

/// `--parity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ParityArg {
    /// No parity bit.
    None,
    /// Odd parity.
    Odd,
    /// Even parity.
    Even,
}

/// `--flow`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FlowArg {
    /// No flow control.
    None,
    /// XON/XOFF.
    Software,
    /// RTS/CTS.
    Hardware,
}

/// What is appended to sent text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Ending {
    /// Nothing.
    None,
    /// `\r`.
    Cr,
    /// `\n`.
    Lf,
    /// `\r\n`.
    Crlf,
}

impl Ending {
    /// The bytes of the line ending.
    pub fn bytes(self) -> &'static [u8] {
        match self {
            Self::None => b"",
            Self::Cr => b"\r",
            Self::Lf => b"\n",
            Self::Crlf => b"\r\n",
        }
    }
}

/// Options of `send`.
#[derive(Debug, Args)]
#[command(group(ArgGroup::new("payload").required(true).args(["text", "hex", "file"])))]
pub struct SendArgs {
    /// Port and its settings.
    #[command(flatten)]
    pub port: PortArgs,
    /// Text to send.
    pub text: Option<String>,
    /// Bytes to send, as hex.
    #[arg(long, value_name = "HEX")]
    pub hex: Option<String>,
    /// A file to send as it is.
    #[arg(long)]
    pub file: Option<PathBuf>,
    /// Line ending after text.
    #[arg(long, value_enum, default_value_t = Ending::Crlf)]
    pub ending: Ending,
    /// Text encoding.
    #[arg(long, default_value = "utf-8")]
    pub encoding: String,
    /// Checksum appended to the data.
    #[arg(long, value_enum)]
    pub checksum: Option<Checksum>,
    /// Wait for a reply matching this regular expression.
    #[arg(long, value_parser = parse_regex, value_name = "REGEX")]
    pub expect: Option<Regex>,
    /// How long to wait (5s; with `--protocol`, 60s for the receiver to start).
    #[arg(long, value_parser = parse_duration)]
    pub timeout: Option<Duration>,
    /// Print what is received.
    #[arg(long)]
    pub print: bool,
    /// Send the file with XMODEM or YMODEM, to a bootloader.
    #[arg(
        long,
        value_enum,
        requires = "file",
        conflicts_with_all = ["text", "hex", "checksum", "expect", "print"]
    )]
    pub protocol: Option<FileProtocol>,
}

/// File transfer protocols of `send --protocol`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum FileProtocol {
    /// 128-byte blocks; CRC or checksum, as the receiver asks.
    Xmodem,
    /// 1024-byte blocks (CRC).
    #[value(name = "xmodem-1k")]
    Xmodem1k,
    /// File name and size, then 1024-byte blocks.
    Ymodem,
    /// File name and size, then 128-byte blocks.
    #[value(name = "ymodem-128")]
    Ymodem128,
}

impl FileProtocol {
    /// The transfer settings, waiting `start` for the receiver.
    pub fn modem(self, start: Duration) -> serialbot_jobs::xmodem::Modem {
        use serialbot_jobs::xmodem::{Modem, Protocol};
        let (protocol, one_k) = match self {
            Self::Xmodem => (Protocol::Xmodem, false),
            Self::Xmodem1k => (Protocol::Xmodem, true),
            Self::Ymodem => (Protocol::Ymodem, true),
            Self::Ymodem128 => (Protocol::Ymodem, false),
        };
        Modem {
            start_timeout: start,
            ..Modem::new(protocol, one_k)
        }
    }
}

/// Options of `monitor`.
#[derive(Debug, Args)]
pub struct MonitorArgs {
    /// Port and its settings.
    #[command(flatten)]
    pub port: PortArgs,
    /// Stop when received data matches this regular expression.
    #[arg(long, value_parser = parse_regex, value_name = "REGEX")]
    pub until: Option<Regex>,
    /// Stop after this long.
    #[arg(long, value_parser = parse_duration)]
    pub duration: Option<Duration>,
    /// One JSON object per line.
    #[arg(long)]
    pub json: bool,
    /// Show bytes as hex.
    #[arg(long)]
    pub hex: bool,
    /// Start each line with the time.
    #[arg(long)]
    pub time: bool,
    /// Also save everything to this file.
    #[arg(long, value_name = "FILE")]
    pub log: Option<PathBuf>,
}

/// Options of `open`.
#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools, reason = "command-line flags")]
pub struct OpenArgs {
    /// Port and its settings.
    #[command(flatten)]
    pub port: PortArgs,
    /// Prefix key: Ctrl plus this letter.
    #[arg(long, default_value_t = 'a', value_parser = parse_escape)]
    pub escape: char,
    /// Start in raw mode.
    #[arg(long)]
    pub raw: bool,
    /// Line ending after each sent line: CR, as a terminal's Enter key.
    #[arg(long, value_enum, default_value_t = Ending::Cr)]
    pub ending: Ending,
    /// Text encoding.
    #[arg(long, default_value = "utf-8")]
    pub encoding: String,
    /// Quit when the device goes away instead of waiting for it.
    #[arg(long)]
    pub no_reconnect: bool,
    /// What Ctrl+A 3 and /reset do.
    #[arg(long, default_value = "dtr=0 rts=1 100ms rts=0", value_name = "STEPS")]
    pub reset_sequence: String,
    /// Save everything to this file from the start (minicom -C).
    #[arg(short = 'C', long, value_name = "FILE")]
    pub log: Option<PathBuf>,
    /// Show received data as hex.
    #[arg(long)]
    pub hex: bool,
    /// Start each line with the time.
    #[arg(long)]
    pub time: bool,
}

/// Options of `tui`: every port opens with the same settings.
#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools, reason = "command-line flags")]
pub struct TuiArgs {
    /// Ports to open at the start (more with `/open`).
    pub ports: Vec<String>,
    /// Baud rate.
    #[arg(short, long, default_value_t = 115_200, value_parser = clap::value_parser!(u32).range(1..))]
    pub baud: u32,
    /// Data bits.
    #[arg(long, default_value_t = 8, value_parser = clap::value_parser!(u8).range(5..=8))]
    pub data: u8,
    /// Parity.
    #[arg(long, value_enum, default_value_t = ParityArg::None)]
    pub parity: ParityArg,
    /// Stop bits.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=2))]
    pub stop: u8,
    /// Flow control.
    #[arg(long, value_enum, default_value_t = FlowArg::None)]
    pub flow: FlowArg,
    /// DTR level right after opening.
    #[arg(long, value_parser = parse_level, value_name = "0|1")]
    pub dtr: Option<bool>,
    /// RTS level right after opening.
    #[arg(long, value_parser = parse_level, value_name = "0|1")]
    pub rts: Option<bool>,
    /// Prefix key: Ctrl plus this letter.
    #[arg(long, default_value_t = 'a', value_parser = parse_escape)]
    pub escape: char,
    /// Line ending after each sent line: CR, as a terminal's Enter key.
    #[arg(long, value_enum, default_value_t = Ending::Cr)]
    pub ending: Ending,
    /// Text encoding.
    #[arg(long, default_value = "utf-8")]
    pub encoding: String,
    /// Do not wait for a device that went away.
    #[arg(long)]
    pub no_reconnect: bool,
    /// What Ctrl+A 3 and /reset do.
    #[arg(long, default_value = "dtr=0 rts=1 100ms rts=0", value_name = "STEPS")]
    pub reset_sequence: String,
    /// Show received data as hex.
    #[arg(long)]
    pub hex: bool,
    /// Start each line with the time.
    #[arg(long)]
    pub time: bool,
    /// Waveform patterns, or `justfloat` for VOFA+ binary frames.
    #[arg(long, value_name = "[NAME=]REGEX|justfloat")]
    pub plot: Vec<String>,
    /// The app's command groups to show.
    #[arg(long, value_name = "FILE")]
    pub groups: Option<String>,
    /// Also run the MCP server, with the AI panel.
    #[arg(long)]
    pub mcp: bool,
    /// The MCP server's preferred port.
    #[arg(long, default_value_t = serialbot_mcp::DEFAULT_PORT, requires = "mcp")]
    pub mcp_port: u16,
    /// Agents' changes: ask (in the AI panel), all or deny.
    #[arg(long, value_enum, default_value_t = Approve::Ask, requires = "mcp")]
    pub approve: Approve,
}

impl TuiArgs {
    /// The settings `port` opens with.
    pub fn port(&self, port: &str) -> PortArgs {
        PortArgs {
            port: port.to_owned(),
            baud: self.baud,
            data: self.data,
            parity: self.parity,
            stop: self.stop,
            flow: self.flow,
            dtr: self.dtr,
            rts: self.rts,
        }
    }
}

fn parse_escape(text: &str) -> Result<char, String> {
    let letter = text.trim().to_ascii_lowercase();
    let letter = letter
        .strip_prefix("ctrl-")
        .or_else(|| letter.strip_prefix("ctrl+"))
        .unwrap_or(&letter);
    let mut chars = letter.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_lowercase() => Ok(c),
        _ => Err(format!("{text:?} is not a letter (e.g. a, t, ctrl-t)")),
    }
}

/// Lets `open` take minicom's `-D <device>` for the port.
pub fn minicom_device(args: Vec<std::ffi::OsString>) -> Vec<std::ffi::OsString> {
    let is_open = args.get(1).is_some_and(|a| a == "open");
    if !is_open {
        return args;
    }
    let mut out = Vec::with_capacity(args.len());
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "-D" {
            if let Some(device) = iter.next() {
                out.push(device);
            }
        } else if let Some(device) = arg
            .to_str()
            .and_then(|a| a.strip_prefix("-D"))
            .filter(|d| !d.is_empty())
        {
            out.push(device.into());
        } else {
            out.push(arg);
        }
    }
    out
}

/// Options of `run`.
#[derive(Debug, Args)]
pub struct RunArgs {
    /// Port and its settings.
    #[command(flatten)]
    pub port: PortArgs,
    /// The script: a text file, or a command export (`.json`) of the app.
    pub script: PathBuf,
    /// Which group of a command export to run.
    #[arg(long)]
    pub group: Option<String>,
    /// Print received data.
    #[arg(long)]
    pub print: bool,
    /// Line ending after `send` text.
    #[arg(long, value_enum, default_value_t = Ending::Crlf)]
    pub ending: Ending,
    /// Text encoding.
    #[arg(long, default_value = "utf-8")]
    pub encoding: String,
}

/// Options of `signal`.
#[derive(Debug, Args)]
pub struct SignalArgs {
    /// Port and its settings.
    #[command(flatten)]
    pub port: PortArgs,
    /// Steps: `dtr=0|1`, `rts=0|1`, a pause like `100ms`, `break[=250ms]`.
    #[arg(value_name = "STEP")]
    pub steps: Vec<String>,
    /// The same steps in one string, run after the others.
    #[arg(long, value_name = "STEPS")]
    pub sequence: Option<String>,
    /// Send a BREAK of this length last.
    #[arg(long = "break", value_parser = parse_duration, value_name = "DURATION")]
    pub r#break: Option<Duration>,
}

fn parse_regex(text: &str) -> Result<Regex, String> {
    Regex::new(text).map_err(|e| e.to_string())
}

/// `--color`: when output is colored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    /// When standard output is a terminal and `NO_COLOR` is not set.
    Auto,
    /// Always, even into a pipe.
    Always,
    /// Never.
    Never,
}

impl ColorChoice {
    /// Whether to color, given whether output is a terminal and whether
    /// `NO_COLOR` is set (to anything non-empty). `always` wins over
    /// `NO_COLOR`, which only changes the default.
    pub fn enabled(self, terminal: bool, no_color: bool) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => terminal && !no_color,
        }
    }
}

/// The parser, with help in `lang`.
pub fn command(lang: Lang) -> clap::Command {
    let s = lang.strings();
    let text = |key: &str| s.help_text(key).unwrap_or_default();
    let template = format!(
        "{{about-with-newline}}\n{}: {{usage}}\n\n{{all-args}}{{after-help}}",
        s.usage
    );
    let localize = |c: clap::Command, prefix: &str| {
        let ids: Vec<String> = c.get_arguments().map(|a| a.get_id().to_string()).collect();
        let mut c = c
            .help_template(template.clone())
            .next_help_heading(s.options)
            .subcommand_help_heading(s.commands);
        for id in ids {
            let key = match id.as_str() {
                "help" | "version" | "color" => id.clone(),
                _ => format!("{prefix}{id}"),
            };
            c = c.mut_arg(id.as_str(), |a| {
                a.help(text(&key))
                    .help_heading(s.options)
                    .hide_default_value(lang == Lang::Zh && key == "color")
            });
        }
        c
    };
    localize_tree(
        localize(Cli::command().about(text("about")), ""),
        "",
        &text,
        &localize,
    )
}

/// Localizes the subcommands of `cmd` (and theirs), keyed `parent.name`.
fn localize_tree(
    mut cmd: clap::Command,
    prefix: &str,
    text: &dyn Fn(&str) -> &'static str,
    localize: &dyn Fn(clap::Command, &str) -> clap::Command,
) -> clap::Command {
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|c| c.get_name().to_owned())
        .collect();
    for name in names {
        let key = format!("{prefix}{name}");
        cmd = cmd.mut_subcommand(name.as_str(), |c| {
            let c = localize(c.about(text(&key)), &format!("{key}."));
            localize_tree(c, &format!("{key}."), text, localize)
        });
    }
    cmd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parser_is_consistent() {
        Cli::command().debug_assert();
        command(Lang::Zh).debug_assert();
    }

    #[test]
    fn every_option_has_help_in_both_languages() {
        fn walk(cmd: &clap::Command, prefix: &str, keys: &mut Vec<String>) {
            for sub in cmd.get_subcommands() {
                let key = format!("{prefix}{}", sub.get_name());
                keys.push(key.clone());
                for arg in sub.get_arguments() {
                    let id = arg.get_id().as_str();
                    if !matches!(id, "help" | "color") {
                        keys.push(format!("{key}.{id}"));
                    }
                }
                walk(sub, &format!("{key}."), keys);
            }
        }
        let mut keys = vec!["about".to_owned()];
        let cli = Cli::command();
        keys.extend(cli.get_arguments().map(|a| a.get_id().to_string()));
        walk(&cli, "", &mut keys);
        for lang in [Lang::En, Lang::Zh] {
            for key in &keys {
                assert!(
                    lang.strings().help_text(key).is_some(),
                    "{lang:?} has no help for {key}"
                );
            }
            assert_eq!(
                lang.strings().help.len(),
                keys.len(),
                "unused help in {lang:?}"
            );
        }
    }

    #[test]
    fn help_follows_the_language() {
        let zh = command(Lang::Zh).render_long_help().to_string();
        assert!(zh.contains("用法"), "{zh}");
        assert!(zh.contains("命令"), "{zh}");
        assert!(zh.contains("显示帮助"), "{zh}");
        let mut zh = command(Lang::Zh);
        let send = zh.find_subcommand_mut("send").unwrap();
        let help = send.render_long_help().to_string();
        assert!(help.contains("等待匹配这个正则表达式的回复"), "{help}");
        let en = command(Lang::En).render_long_help().to_string();
        assert!(en.contains("Usage"), "{en}");
        assert!(en.contains("List the serial ports"), "{en}");
    }

    #[test]
    fn colors_follow_the_terminal_and_no_color() {
        assert!(ColorChoice::Auto.enabled(true, false));
        assert!(!ColorChoice::Auto.enabled(false, false));
        assert!(!ColorChoice::Auto.enabled(true, true));
        assert!(ColorChoice::Always.enabled(false, true));
        assert!(!ColorChoice::Never.enabled(true, false));
    }

    fn send_args(args: &[&str]) -> Result<SendArgs, clap::Error> {
        let cli = Cli::try_parse_from(["x", "send", "p"].iter().chain(args))?;
        let Command::Send(send) = cli.command else {
            panic!("not send");
        };
        Ok(send)
    }

    #[test]
    fn port_options_become_the_serial_settings() {
        let send = send_args(&[
            "AT", "-b", "9600", "--data", "7", "--parity", "even", "--stop", "2", "--flow",
            "hardware", "--dtr", "0", "--rts", "off",
        ])
        .unwrap();
        let c = send.port.config();
        assert_eq!((c.path.as_str(), c.baud_rate), ("p", 9600));
        assert_eq!(c.data_bits, DataBits::Seven);
        assert_eq!(c.parity, Parity::Even);
        assert_eq!(c.stop_bits, StopBits::Two);
        assert_eq!(c.flow_control, FlowControl::Hardware);
        assert_eq!((c.dtr_on_open, c.rts_on_open), (Some(false), Some(false)));
        let odd = send_args(&["--hex", "01", "--parity", "odd"]).unwrap();
        assert_eq!(odd.port.config().parity, Parity::Odd);
        assert_eq!(odd.port.config().flow_control, FlowControl::None);
        let soft = send_args(&["x", "--flow", "software"]).unwrap();
        assert_eq!(soft.port.config().flow_control, FlowControl::Software);
    }

    #[test]
    fn send_needs_exactly_one_kind_of_data_and_valid_values() {
        assert!(send_args(&[]).is_err(), "nothing to send");
        assert!(send_args(&["AT", "--hex", "01"]).is_err(), "two kinds");
        assert!(send_args(&["--data", "9", "AT"]).is_err());
        assert!(send_args(&["--stop", "3", "AT"]).is_err());
        assert!(send_args(&["--dtr", "maybe", "AT"]).is_err());
        assert!(send_args(&["--expect", "(", "AT"]).is_err());
        assert!(send_args(&["--timeout", "5", "AT"]).is_err());
        assert!(send_args(&["AT"]).is_ok());
        assert_eq!(Ending::None.bytes(), b"");
        assert_eq!(Ending::Cr.bytes(), b"\r");
        assert_eq!(Ending::Lf.bytes(), b"\n");
        assert_eq!(Ending::Crlf.bytes(), b"\r\n");
    }
}
