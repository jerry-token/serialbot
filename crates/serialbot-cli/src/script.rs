// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Scripts for `run` and the interactive `/run`: the engine shared with the
//! app ([`serialbot_jobs::script`]), with the command line's line endings.

pub use serialbot_jobs::script::{Group, Item, Progress, Stop, load_groups, parse_groups, run};

use crate::args::Ending;

impl From<Ending> for serialbot_jobs::script::Ending {
    fn from(ending: Ending) -> Self {
        match ending {
            Ending::None => Self::None,
            Ending::Cr => Self::Cr,
            Ending::Lf => Self::Lf,
            Ending::Crlf => Self::Crlf,
        }
    }
}

/// Parses a text script: sends get `ending` until an `ending` line changes
/// it and are encoded in `encoding`. Errors name the line.
pub fn parse_text(text: &str, ending: Ending, encoding: &str) -> Result<Vec<Item>, String> {
    serialbot_jobs::script::parse_text(text, ending.into(), encoding)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_endings_carry_over() {
        for (ending, bytes) in [
            (Ending::None, &b"x"[..]),
            (Ending::Cr, b"x\r"),
            (Ending::Lf, b"x\n"),
            (Ending::Crlf, b"x\r\n"),
        ] {
            let items = parse_text("send x", ending, "utf-8").unwrap();
            assert!(
                matches!(&items[0].step, serialbot_jobs::script::Step::Send(b) if b == bytes),
                "{ending:?}"
            );
        }
    }
}
