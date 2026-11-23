//! Where results and messages go: results to standard output, errors to
//! standard error, in the user's language, colored only when wanted.
//!
//! This is the only module that writes to the terminal; the workspace
//! forbids `print!` everywhere else.

use std::io::Write;

use crate::i18n::{Lang, Strings};

/// A text style for terminals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Less important text (timestamps, headings).
    Dim,
    /// Emphasis.
    Bold,
    /// Errors.
    Red,
}

impl Style {
    fn code(self) -> &'static str {
        match self {
            Self::Dim => "2",
            Self::Bold => "1",
            Self::Red => "31",
        }
    }
}

/// The two output streams of a run.
pub struct Output<'a> {
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
    lang: Lang,
    color: bool,
}

impl std::fmt::Debug for Output<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Output")
            .field("lang", &self.lang)
            .field("color", &self.color)
            .finish_non_exhaustive()
    }
}

impl<'a> Output<'a> {
    /// Output to `out` and `err`; `color` turns ANSI styles on.
    pub fn new(out: &'a mut dyn Write, err: &'a mut dyn Write, lang: Lang, color: bool) -> Self {
        Self {
            out,
            err,
            lang,
            color,
        }
    }

    /// The messages in the user's language.
    pub fn strings(&self) -> &'static Strings {
        self.lang.strings()
    }

    /// `text` in `style` when colors are on, else unchanged.
    pub fn paint(&self, style: Style, text: &str) -> String {
        if self.color {
            format!("\x1b[{}m{text}\x1b[0m", style.code())
        } else {
            text.to_owned()
        }
    }

    /// Writes one line of results. A closed pipe (`| head`) is not an
    /// error worth reporting, so write errors are ignored.
    pub fn line(&mut self, text: &str) {
        let _ = writeln!(self.out, "{text}");
    }

    /// Writes received bytes unchanged.
    pub fn bytes(&mut self, data: &[u8]) {
        let _ = self.out.write_all(data);
        let _ = self.out.flush();
    }

    /// Writes one JSON value on its own line.
    pub fn json(&mut self, value: &serde_json::Value) {
        self.line(&value.to_string());
    }

    /// Writes a note (not an error) to standard error, so it never mixes
    /// with results piped elsewhere.
    pub fn error_note(&mut self, text: &str) {
        let _ = writeln!(self.err, "{text}");
    }

    /// Writes `error: <message>` to standard error.
    pub fn error(&mut self, message: &str) {
        let prefix = self.paint(Style::Red, self.strings().error);
        let _ = writeln!(self.err, "{prefix}: {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_and_errors_go_to_their_streams() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut o = Output::new(&mut out, &mut err, Lang::Zh, false);
        o.line("a");
        o.bytes(b"b\n");
        o.json(&serde_json::json!({"k": 1}));
        o.error("坏了");
        assert!(format!("{o:?}").contains("Zh"));
        assert_eq!(String::from_utf8(out).unwrap(), "a\nb\n{\"k\":1}\n");
        assert_eq!(String::from_utf8(err).unwrap(), "错误: 坏了\n");
    }

    #[test]
    fn styles_only_with_colors() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let plain = Output::new(&mut out, &mut err, Lang::En, false);
        assert_eq!(plain.paint(Style::Bold, "x"), "x");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let colored = Output::new(&mut out, &mut err, Lang::En, true);
        assert_eq!(colored.paint(Style::Dim, "x"), "\x1b[2mx\x1b[0m");
        assert_eq!(colored.paint(Style::Bold, "x"), "\x1b[1mx\x1b[0m");
        assert_eq!(colored.paint(Style::Red, "x"), "\x1b[31mx\x1b[0m");
    }
}
