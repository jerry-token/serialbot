// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! The input box of the interactive terminal: a small multi-line editor
//! with history, as in a shell.
//!
//! Positions are counted in characters, never bytes, so Chinese text and
//! other multi-byte characters move and delete as one unit.

/// How many sent lines the history keeps (oldest dropped first).
pub const HISTORY_LIMIT: usize = 500;

/// The text being typed, its cursor and the history of sent lines.
#[derive(Debug, Default, Clone)]
pub struct Editor {
    text: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    /// Position while browsing the history (`None`: editing a new line).
    browsing: Option<usize>,
    /// What was typed before browsing started, restored at the bottom.
    draft: Vec<char>,
}

impl Editor {
    /// The text as typed.
    pub fn text(&self) -> String {
        self.text.iter().collect()
    }

    /// The cursor, in characters from the start.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Whether nothing is typed.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The sent lines, oldest first.
    pub fn history(&self) -> &[String] {
        &self.history
    }

    /// Inserts `c` at the cursor.
    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += 1;
        self.browsing = None;
    }

    /// Inserts `s` at the cursor (a paste).
    pub fn insert_str(&mut self, s: &str) {
        for c in s.chars() {
            self.insert(c);
        }
    }

    /// Deletes the character before the cursor; nothing at the start.
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.text.remove(self.cursor);
        }
    }

    /// Deletes the character under the cursor; nothing at the end.
    pub fn delete(&mut self) {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    /// Deletes from the start of the current line to the cursor (Ctrl+U).
    pub fn delete_to_line_start(&mut self) {
        let start = self.line_start();
        self.text.drain(start..self.cursor);
        self.cursor = start;
    }

    /// Deletes the word before the cursor (Ctrl+W).
    pub fn delete_word(&mut self) {
        let mut start = self.cursor;
        while start > 0 && self.text[start - 1].is_whitespace() {
            start -= 1;
        }
        while start > 0 && !self.text[start - 1].is_whitespace() {
            start -= 1;
        }
        self.text.drain(start..self.cursor);
        self.cursor = start;
    }

    /// Moves one character left.
    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Moves one character right.
    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.text.len());
    }

    /// Moves to the start of the current line.
    pub fn home(&mut self) {
        self.cursor = self.line_start();
    }

    /// Moves to the end of the current line.
    pub fn end(&mut self) {
        self.cursor = self.text[self.cursor..]
            .iter()
            .position(|c| *c == '\n')
            .map_or(self.text.len(), |i| self.cursor + i);
    }

    fn line_start(&self) -> usize {
        self.text[..self.cursor]
            .iter()
            .rposition(|c| *c == '\n')
            .map_or(0, |i| i + 1)
    }

    /// Whether the text has more than one line.
    pub fn is_multiline(&self) -> bool {
        self.text.contains(&'\n')
    }

    /// Shows the previous sent line (Up). Returns false at the oldest.
    pub fn history_up(&mut self) -> bool {
        let next = match self.browsing {
            None if self.history.is_empty() => return false,
            None => {
                self.draft = self.text.clone();
                self.history.len() - 1
            }
            Some(0) => return false,
            Some(i) => i - 1,
        };
        self.show_history(next);
        true
    }

    /// Shows the next sent line, then what was typed before browsing
    /// (Down). Returns false when not browsing.
    pub fn history_down(&mut self) -> bool {
        match self.browsing {
            None => false,
            Some(i) if i + 1 < self.history.len() => {
                self.show_history(i + 1);
                true
            }
            Some(_) => {
                self.text = std::mem::take(&mut self.draft);
                self.cursor = self.text.len();
                self.browsing = None;
                true
            }
        }
    }

    fn show_history(&mut self, index: usize) {
        self.text = self.history[index].chars().collect();
        self.cursor = self.text.len();
        self.browsing = Some(index);
    }

    /// The newest sent line starting with `prefix` (before `before`, an
    /// index into the history), for Ctrl+R; its index and text.
    pub fn search(&self, needle: &str, before: Option<usize>) -> Option<(usize, &str)> {
        let end = before.unwrap_or(self.history.len());
        self.history[..end]
            .iter()
            .enumerate()
            .rev()
            .find(|(_, line)| line.contains(needle))
            .map(|(i, line)| (i, line.as_str()))
    }

    /// Replaces the text with history entry `index` (accepting a search).
    pub fn take_from_history(&mut self, index: usize) {
        if index < self.history.len() {
            self.show_history(index);
            self.browsing = None;
        }
    }

    /// Clears the text (Ctrl+C, Esc).
    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.browsing = None;
    }

    /// Returns the text and clears it; a non-empty line different from the
    /// last one is added to the history.
    pub fn submit(&mut self) -> String {
        let line = self.text();
        if !line.is_empty() && self.history.last() != Some(&line) {
            self.history.push(line.clone());
            if self.history.len() > HISTORY_LIMIT {
                self.history.remove(0);
            }
        }
        self.clear();
        self.draft.clear();
        line
    }

    /// Replaces the history (loaded at start).
    pub fn set_history(&mut self, history: Vec<String>) {
        let skip = history.len().saturating_sub(HISTORY_LIMIT);
        self.history = history.into_iter().skip(skip).collect();
        self.browsing = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> Editor {
        let mut e = Editor::default();
        e.insert_str(s);
        e
    }

    #[test]
    fn edits_by_character_not_byte() {
        let mut e = typed("温度AT");
        assert_eq!(e.cursor(), 4);
        e.left();
        e.left();
        e.backspace();
        assert_eq!(e.text(), "温AT");
        e.delete();
        assert_eq!(e.text(), "温T");
        e.home();
        e.backspace();
        assert_eq!(e.text(), "温T", "nothing before the start");
        e.end();
        e.delete();
        assert_eq!(e.text(), "温T", "nothing after the end");
        e.right();
        assert_eq!(e.cursor(), 2, "the cursor stays at the end");
        assert!(!e.is_empty());
    }

    #[test]
    fn lines_have_their_own_home_and_end() {
        let mut e = typed("first\nsecond");
        assert!(e.is_multiline());
        e.home();
        assert_eq!(e.cursor(), 6);
        e.left();
        e.home();
        assert_eq!(e.cursor(), 0);
        e.end();
        assert_eq!(e.cursor(), 5);
        e.right();
        e.right();
        e.delete_to_line_start();
        assert_eq!(e.text(), "first\necond");
    }

    #[test]
    fn deletes_words() {
        let mut e = typed("AT+CWJAP  ssid ");
        e.delete_word();
        assert_eq!(e.text(), "AT+CWJAP  ");
        e.delete_word();
        assert_eq!(e.text(), "");
        e.delete_word();
        assert_eq!(e.text(), "");
    }

    #[test]
    fn history_browses_and_restores_the_draft() {
        let mut e = Editor::default();
        assert!(!e.history_up(), "no history yet");
        for line in ["AT", "AT+GMR", "AT+GMR", ""] {
            e.insert_str(line);
            assert_eq!(e.submit(), line);
        }
        assert_eq!(e.history(), ["AT", "AT+GMR"], "no repeats, no empty lines");
        e.insert_str("draft");
        assert!(e.history_up());
        assert_eq!(e.text(), "AT+GMR");
        assert!(e.history_up());
        assert_eq!(e.text(), "AT");
        assert!(!e.history_up(), "the oldest");
        assert!(e.history_down());
        assert_eq!(e.text(), "AT+GMR");
        assert!(e.history_down());
        assert_eq!(e.text(), "draft");
        assert!(!e.history_down(), "not browsing any more");
    }

    #[test]
    fn history_search_finds_newest_first() {
        let mut e = Editor::default();
        e.set_history(vec!["AT+CWLAP".into(), "led on".into(), "AT+GMR".into()]);
        assert_eq!(e.search("AT", None), Some((2, "AT+GMR")));
        assert_eq!(e.search("AT", Some(2)), Some((0, "AT+CWLAP")));
        assert_eq!(e.search("nope", None), None);
        e.take_from_history(1);
        assert_eq!(e.text(), "led on");
        e.take_from_history(99);
        assert_eq!(e.text(), "led on");
    }

    #[test]
    fn history_is_limited() {
        let mut e = Editor::default();
        e.set_history((0..HISTORY_LIMIT + 3).map(|i| i.to_string()).collect());
        assert_eq!(e.history().len(), HISTORY_LIMIT);
        assert_eq!(e.history()[0], "3");
        e.insert_str("new");
        e.submit();
        assert_eq!(e.history().len(), HISTORY_LIMIT);
        assert_eq!(e.history()[0], "4");
        e.insert_str("x");
        e.clear();
        assert!(e.is_empty());
    }
}
