//! The command line: subcommands and their options, as plain data.
//!
//! Help texts are written in English here and replaced by
//! [`command`] when the language is Chinese, so `--help` follows the
//! system language like every other message.

use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand, ValueEnum};

use crate::i18n::Lang;

/// `serialbot-cli [--color …] <command> …`
#[derive(Debug, Parser)]
#[command(
    name = "serialbot-cli",
    version,
    about = "SerialBot in the terminal: list ports, send, monitor.",
    disable_help_flag = true,
    disable_version_flag = true,
    disable_help_subcommand = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// When to use colors: auto, always or never.
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
    /// List the serial ports (name, chip, VID/PID, serial number).
    Ports(PortsArgs),
}

/// Options of `ports`.
#[derive(Debug, Args)]
pub struct PortsArgs {
    /// One JSON object per line.
    #[arg(long)]
    pub json: bool,
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
    let template = format!(
        "{{about-with-newline}}\n{}: {{usage}}\n\n{{all-args}}{{after-help}}",
        s.usage
    );
    let localize_common = |c: clap::Command| {
        c.help_template(template.clone())
            .next_help_heading(s.options)
            .subcommand_help_heading(s.commands)
            .mut_arg("help", |a| a.help(s.help_flag).help_heading(s.options))
            .mut_arg("color", |a| {
                a.help(s.color_flag)
                    .help_heading(s.options)
                    .hide_default_value(lang == Lang::Zh)
            })
    };
    let cmd = Cli::command().about(s.about).mut_arg("version", |a| {
        a.help(s.version_flag).help_heading(s.options)
    });
    localize_common(cmd).mut_subcommand("ports", |c| {
        c.about(s.ports_about)
            .help_template(template.clone())
            .mut_arg("json", |a| a.help(s.json_flag).help_heading(s.options))
    })
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
    fn help_follows_the_language() {
        let zh = command(Lang::Zh).render_long_help().to_string();
        assert!(zh.contains("用法"), "{zh}");
        assert!(zh.contains("命令"), "{zh}");
        assert!(zh.contains("显示帮助"), "{zh}");
        let mut zh = command(Lang::Zh);
        let ports = zh.find_subcommand_mut("ports").unwrap();
        let help = ports.render_long_help().to_string();
        assert!(help.contains("每行输出一个 JSON 对象"), "{help}");
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
}
