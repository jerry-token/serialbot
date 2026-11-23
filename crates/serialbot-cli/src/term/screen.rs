// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Drawing the interactive terminal: received lines scroll up like any
//! program output (they stay in the terminal's own scrollback, can be
//! copied and remain after quitting), and only the bottom rows (the
//! unfinished line, the input box and the status bar) are redrawn.
//!
//! To print, the bottom rows are erased, the lines are written, and the
//! bottom is drawn again below them, inside one synchronized update so the
//! terminal shows no flicker.

use std::io::{self, Write};

use crossterm::cursor::{MoveToColumn, MoveUp};
use crossterm::terminal::{BeginSynchronizedUpdate, Clear, ClearType, EndSynchronizedUpdate};
use crossterm::{QueueableCommand, style::Print};
use unicode_width::UnicodeWidthChar;

/// What the bottom rows show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bottom {
    /// The unfinished received line, if any.
    pub pending: Option<String>,
    /// The input box, one string per line, without the prompt.
    pub input: Vec<String>,
    /// The cursor in the input box: line and character index.
    pub cursor: (usize, usize),
    /// The status bar (already styled).
    pub status: String,
    /// Shown before the first input line.
    pub prompt: String,
}

/// The display width of `text`, ignoring color sequences.
pub fn width(text: &str) -> usize {
    let mut total = 0;
    let mut in_escape = false;
    for c in text.chars() {
        if in_escape {
            in_escape = !('\u{40}'..='\u{7e}').contains(&c) || c == '[';
            continue;
        }
        if c == '\u{1b}' {
            in_escape = true;
            continue;
        }
        total += c.width().unwrap_or(0);
    }
    total
}

/// The rows a line of `cols` display columns takes on a screen `width`
/// wide (an empty line still takes one).
fn rows(cols: usize, width: usize) -> usize {
    cols.div_ceil(width.max(1)).max(1)
}

/// `text` cut to at most `max` columns (color sequences are kept, and the
/// color is reset at the cut).
pub fn truncate(text: &str, max: usize) -> String {
    if width(text) <= max {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    let mut in_escape = false;
    for c in text.chars() {
        if in_escape || c == '\u{1b}' {
            in_escape = if c == '\u{1b}' {
                true
            } else {
                !('\u{40}'..='\u{7e}').contains(&c) || c == '['
            };
            out.push(c);
            continue;
        }
        let w = c.width().unwrap_or(0);
        if used + w > max {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push_str("\x1b[0m");
    out
}

/// The bottom rows on screen and where the cursor is in them.
#[derive(Debug, Default)]
pub struct Screen {
    width: usize,
    /// Rows above the cursor that belong to the drawn bottom.
    cursor_row: usize,
    drawn: bool,
}

impl Screen {
    /// A screen `width` columns wide with nothing drawn yet.
    pub fn new(width: u16) -> Self {
        Self {
            width: usize::from(width.max(1)),
            cursor_row: 0,
            drawn: false,
        }
    }

    /// Changes the width (the terminal was resized). The bottom is erased
    /// as drawn with the old width, which the terminal may have reflowed;
    /// the next draw starts clean.
    pub fn resize(&mut self, width: u16) {
        self.width = usize::from(width.max(1));
    }

    /// Erases the bottom rows, leaving the cursor where they started.
    pub fn erase(&mut self, out: &mut impl Write) -> io::Result<()> {
        if self.drawn {
            out.queue(MoveToColumn(0))?;
            if self.cursor_row > 0 {
                out.queue(MoveUp(u16::try_from(self.cursor_row).unwrap_or(u16::MAX)))?;
            }
            out.queue(Clear(ClearType::FromCursorDown))?;
        }
        self.drawn = false;
        self.cursor_row = 0;
        Ok(())
    }

    /// Prints `lines` above the bottom, then draws the bottom again.
    pub fn print(
        &mut self,
        out: &mut impl Write,
        lines: &[String],
        bottom: &Bottom,
    ) -> io::Result<()> {
        out.queue(BeginSynchronizedUpdate)?;
        self.erase(out)?;
        for line in lines {
            out.queue(Print(line))?;
            out.queue(Print("\r\n"))?;
        }
        self.draw_bottom(out, bottom)?;
        out.queue(EndSynchronizedUpdate)?;
        out.flush()
    }

    /// Draws the bottom again (the input or status changed).
    pub fn draw(&mut self, out: &mut impl Write, bottom: &Bottom) -> io::Result<()> {
        self.print(out, &[], bottom)
    }

    fn draw_bottom(&mut self, out: &mut impl Write, bottom: &Bottom) -> io::Result<()> {
        let mut rows_before_cursor = 0;
        let mut cursor_col = 0;
        let mut total_rows = 0;
        let mut first = true;
        let mut write_row = |out: &mut dyn Write, text: &str| -> io::Result<usize> {
            if !first {
                out.write_all(b"\r\n")?;
            }
            first = false;
            out.write_all(text.as_bytes())?;
            Ok(rows(width(text), self.width))
        };
        if let Some(pending) = &bottom.pending {
            total_rows += write_row(out, pending)?;
        }
        let indent = " ".repeat(width(&bottom.prompt));
        for (i, line) in bottom.input.iter().enumerate() {
            let lead = if i == 0 { &bottom.prompt } else { &indent };
            let text = format!("{lead}{line}");
            if i == bottom.cursor.0 {
                let before: String = line.chars().take(bottom.cursor.1).collect();
                let col = width(lead) + width(&before);
                rows_before_cursor = total_rows + col / self.width;
                cursor_col = col % self.width;
            }
            total_rows += write_row(out, &text)?;
        }
        let status = truncate(&bottom.status, self.width.saturating_sub(1));
        total_rows += write_row(out, &status)?;
        // The cursor is now on the last row; move it into the input box.
        let up = total_rows - 1 - rows_before_cursor;
        if up > 0 {
            out.queue(MoveUp(u16::try_from(up).unwrap_or(u16::MAX)))?;
        }
        out.queue(MoveToColumn(u16::try_from(cursor_col).unwrap_or(u16::MAX)))?;
        self.cursor_row = rows_before_cursor;
        self.drawn = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A terminal emulator 20 columns by 8 rows that applies the output.
    fn terminal() -> vt100::Parser {
        vt100::Parser::new(8, 20, 100)
    }

    fn bottom(input: &[&str], cursor: (usize, usize)) -> Bottom {
        Bottom {
            pending: None,
            input: input.iter().map(|s| (*s).to_owned()).collect(),
            cursor,
            status: "ttyUSB0 115200".into(),
            prompt: "› ".into(),
        }
    }

    fn rows_of(vt: &vt100::Parser) -> Vec<String> {
        vt.screen()
            .rows(0, 20)
            .map(|r| r.trim_end().to_owned())
            .collect()
    }

    #[test]
    fn printed_lines_scroll_above_the_bottom() {
        let mut vt = terminal();
        let mut screen = Screen::new(20);
        let mut out = Vec::new();
        screen.draw(&mut out, &bottom(&["AT"], (0, 2))).unwrap();
        screen
            .print(
                &mut out,
                &["boot".into(), "ready".into()],
                &bottom(&["AT"], (0, 2)),
            )
            .unwrap();
        vt.process(&out);
        assert_eq!(
            rows_of(&vt)[..4],
            ["boot", "ready", "› AT", "ttyUSB0 115200"]
        );
        assert_eq!(vt.screen().cursor_position(), (2, 4), "after AT in the box");
    }

    #[test]
    fn the_pending_line_and_a_multi_line_input_are_drawn() {
        let mut vt = terminal();
        let mut screen = Screen::new(20);
        let mut out = Vec::new();
        let mut b = bottom(&["first", "second"], (1, 3));
        b.pending = Some("uart:~$ ".into());
        screen.draw(&mut out, &b).unwrap();
        // Redrawing after an edit replaces the old bottom completely.
        b.input = vec!["x".into()];
        b.cursor = (0, 1);
        b.pending = None;
        screen.draw(&mut out, &b).unwrap();
        vt.process(&out);
        assert_eq!(rows_of(&vt)[..3], ["› x", "ttyUSB0 115200", ""]);
        assert_eq!(vt.screen().cursor_position(), (0, 3));
    }

    #[test]
    fn the_cursor_follows_wide_characters_and_wrapping() {
        let mut vt = terminal();
        let mut screen = Screen::new(20);
        let mut out = Vec::new();
        // 2 + 2*12 = 26 columns: the input wraps onto a second row.
        let text = "温度温度温度温度温度温度";
        screen.draw(&mut out, &bottom(&[text], (0, 10))).unwrap();
        vt.process(&out);
        assert_eq!(
            vt.screen().cursor_position(),
            (1, 2),
            "(2 + 20) columns: row 2, column 2"
        );
        assert_eq!(rows_of(&vt)[2], "ttyUSB0 115200");
        out.clear();
        screen.erase(&mut out).unwrap();
        vt.process(&out);
        assert!(
            rows_of(&vt).iter().all(String::is_empty),
            "{:?}",
            rows_of(&vt)
        );
        screen.erase(&mut out).unwrap();
    }

    #[test]
    fn long_status_bars_are_cut_to_the_width() {
        let mut vt = terminal();
        let mut screen = Screen::new(20);
        screen.resize(10);
        let mut out = Vec::new();
        let mut b = bottom(&[""], (0, 0));
        b.status = "\x1b[2mvery long status bar\x1b[0m".into();
        screen.draw(&mut out, &b).unwrap();
        vt.process(&out);
        assert_eq!(rows_of(&vt)[1], "very long");
        assert_eq!(width("\x1b[1;32m温度\x1b[0m ok"), 7);
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("温度温度", 5), "温度\x1b[0m");
    }
}
