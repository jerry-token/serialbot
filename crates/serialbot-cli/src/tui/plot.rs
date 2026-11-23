// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! The waveform: numbers taken from received lines, drawn as up to
//! [`MAX_SERIES`] curves.
//!
//! By default numbers are found as the App's plot finds them: `name=value`
//! or `name: value` pairs anywhere in a line, else a line of nothing but
//! numbers gives `ch1`, `ch2`, …. Patterns (`--plot`, `/plot`) replace
//! that: a regular expression whose capture groups are the numbers, named
//! `name=regex`, by `(?P<name>…)` groups, or after the pattern.
//!
//! Curves are told apart by color *and* point shape, in a fixed order:
//! cyan •, yellow ×, magenta +, green ▪, blue ◆, white ○. Only the eight
//! basic terminal colors are used, so dark and light themes both work;
//! red is kept for errors (and red–green color blindness). Without color
//! the shapes and the legend still tell the curves apart.

use std::collections::VecDeque;
use std::sync::LazyLock;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use regex::Regex;

/// At most this many curves; later names are ignored.
pub const MAX_SERIES: usize = 6;

/// Samples kept per curve.
pub const CAPACITY: usize = 4096;

/// Colors and point shapes, in the order curves get them.
pub const PALETTE: [(Color, char); MAX_SERIES] = [
    (Color::Cyan, '•'),
    (Color::Yellow, '×'),
    (Color::Magenta, '+'),
    (Color::Green, '▪'),
    (Color::Blue, '◆'),
    (Color::White, '○'),
];

/// Colors `/plot color` accepts (red too, though it is not given out).
pub const COLOR_NAMES: [(&str, Color); 7] = [
    ("cyan", Color::Cyan),
    ("yellow", Color::Yellow),
    ("magenta", Color::Magenta),
    ("green", Color::Green),
    ("blue", Color::Blue),
    ("white", Color::White),
    ("red", Color::Red),
];

const NUMBER: &str = r"[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?";

static PAIR: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(&format!(
        r"([A-Za-z_][\w.\[\]]*)\s*[=:]\s*({NUMBER})(?:[^\w.]|$)"
    ))
    .ok()
});
static BARE_LINE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^\s*{NUMBER}(?:\s*[,;\s]\s*{NUMBER})*\s*[,;]?\s*$"
    ))
    .ok()
});
static BARE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(NUMBER).ok());
static ESCAPES: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07\x1b]*(?:\x07|\x1b\\)|[@-_])").ok()
});

/// The numbers of one line found the App's way (see the module docs).
pub fn auto_sample(line: &str) -> Vec<(String, f64)> {
    let (Some(pair), Some(bare_line), Some(bare)) = (&*PAIR, &*BARE_LINE, &*BARE) else {
        return Vec::new();
    };
    let mut sample: Vec<(String, f64)> = Vec::new();
    for m in pair.captures_iter(line) {
        let value: Option<f64> = m.get(2).and_then(|v| v.as_str().parse().ok());
        if let (Some(name), Some(value)) = (m.get(1), value.filter(|v| v.is_finite())) {
            set(&mut sample, name.as_str(), value);
        }
    }
    if !sample.is_empty() || !bare_line.is_match(line) {
        return sample;
    }
    for (i, m) in bare.find_iter(line).enumerate() {
        if let Some(value) = m.as_str().parse::<f64>().ok().filter(|v| v.is_finite()) {
            sample.push((format!("ch{}", i + 1), value));
        }
    }
    sample
}

/// Sets `name` (its last value wins).
fn set(sample: &mut Vec<(String, f64)>, name: &str, value: f64) {
    match sample.iter_mut().find(|(n, _)| n == name) {
        Some(slot) => slot.1 = value,
        None => sample.push((name.to_owned(), value)),
    }
}

/// A user pattern: a regular expression and the curve names of its groups.
#[derive(Debug, Clone)]
pub struct Pattern {
    regex: Regex,
    names: Vec<String>,
}

impl Pattern {
    /// `name=regex`, or a bare regex (then group names, or `pattern`,
    /// `pattern2`, … from the text before the first group). A pattern
    /// without groups takes its whole match as the number.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let (label, source) = match spec.split_once('=') {
            Some((name, rest))
                if !name.is_empty()
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && !rest.is_empty() =>
            {
                (Some(name.to_owned()), rest)
            }
            _ => (None, spec),
        };
        let regex = Regex::new(source).map_err(|e| format!("--plot {spec:?}: {e}"))?;
        let groups = regex.captures_len() - 1;
        let base = label.unwrap_or_else(|| default_name(source));
        let names = if groups == 0 {
            vec![base]
        } else {
            regex
                .capture_names()
                .skip(1)
                .enumerate()
                .map(|(i, name)| match (name, i) {
                    (Some(n), _) => n.to_owned(),
                    (None, 0) => base.clone(),
                    (None, i) => format!("{base}{}", i + 1),
                })
                .collect()
        };
        Ok(Self { regex, names })
    }

    /// The numbers this pattern finds in `line` (the first match).
    fn sample(&self, line: &str, into: &mut Vec<(String, f64)>) {
        let Some(caps) = self.regex.captures(line) else {
            return;
        };
        let groups: Vec<Option<regex::Match<'_>>> = if caps.len() == 1 {
            vec![caps.get(0)]
        } else {
            caps.iter().skip(1).collect()
        };
        for (name, group) in self.names.iter().zip(groups) {
            let value = group.and_then(|g| g.as_str().trim().parse::<f64>().ok());
            if let Some(value) = value.filter(|v| v.is_finite()) {
                set(into, name, value);
            }
        }
    }
}

/// `temp` from `temp=([0-9.]+)`: the letters before the first group.
fn default_name(source: &str) -> String {
    let name: String = source
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        "value".to_owned()
    } else {
        name
    }
}

/// One curve.
#[derive(Debug, Clone)]
pub struct Series {
    /// Its name, from the line or the pattern.
    pub name: String,
    /// Its color (changed by `/plot color`).
    pub color: Color,
    /// Its point shape.
    pub marker: char,
    /// One value per sample, `None` where the line did not have it.
    pub values: VecDeque<Option<f64>>,
}

impl Series {
    /// The newest value it has.
    pub fn latest(&self) -> Option<f64> {
        self.values.iter().rev().find_map(|v| *v)
    }
}

/// The curves of one session.
#[derive(Debug, Clone, Default)]
pub struct Plot {
    /// Patterns; empty means found the App's way.
    pub patterns: Vec<Pattern>,
    /// The curves, in the order they appeared.
    pub series: Vec<Series>,
    /// Samples taken so far (lines that had numbers), all curves aligned.
    pub samples: usize,
    pending: Vec<u8>,
}

impl Plot {
    /// A plot finding numbers with `patterns` (none: the App's way).
    pub fn new(patterns: Vec<Pattern>) -> Self {
        Self {
            patterns,
            ..Self::default()
        }
    }

    /// Adds received bytes; every finished line (CR, LF or CRLF) that has
    /// numbers is one sample.
    pub fn push(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if b == b'\n' || b == b'\r' {
                let line = std::mem::take(&mut self.pending);
                if !line.is_empty() {
                    self.line(&String::from_utf8_lossy(&line));
                }
            } else {
                self.pending.push(b);
            }
        }
    }

    /// Takes one line of text.
    pub fn line(&mut self, line: &str) {
        let text = match &*ESCAPES {
            Some(escapes) => escapes.replace_all(line, ""),
            None => line.into(),
        };
        let sample = if self.patterns.is_empty() {
            auto_sample(&text)
        } else {
            let mut sample = Vec::new();
            for pattern in &self.patterns {
                pattern.sample(&text, &mut sample);
            }
            sample
        };
        self.add(&sample);
    }

    fn add(&mut self, sample: &[(String, f64)]) {
        if sample.is_empty() {
            return;
        }
        for (name, _) in sample {
            if self.series.len() < MAX_SERIES && !self.series.iter().any(|s| &s.name == name) {
                let (color, marker) = PALETTE[self.series.len()];
                let missing = self.samples.min(CAPACITY);
                self.series.push(Series {
                    name: name.clone(),
                    color,
                    marker,
                    values: std::iter::repeat_n(None, missing).collect(),
                });
            }
        }
        for series in &mut self.series {
            let value = sample
                .iter()
                .find(|(n, _)| *n == series.name)
                .map(|(_, v)| *v);
            if series.values.len() == CAPACITY {
                series.values.pop_front();
            }
            series.values.push_back(value);
        }
        self.samples += 1;
    }

    /// Forgets every sample and curve (patterns stay).
    pub fn clear(&mut self) {
        self.series.clear();
        self.samples = 0;
        self.pending.clear();
    }

    /// Gives curve `name` the color `color` (a [`COLOR_NAMES`] name).
    /// The reason comes back if either is unknown.
    pub fn set_color(&mut self, name: &str, color: &str) -> Result<(), String> {
        let color = COLOR_NAMES
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(color))
            .map(|(_, c)| *c)
            .ok_or_else(|| {
                let names: Vec<&str> = COLOR_NAMES.iter().map(|(n, _)| *n).collect();
                format!("{color}: {}", names.join(", "))
            })?;
        let Some(i) = self.series.iter().position(|s| s.name == name) else {
            let names: Vec<&str> = self.series.iter().map(|s| s.name.as_str()).collect();
            return Err(format!("{name}: {}", names.join(", ")));
        };
        self.series[i].color = color;
        Ok(())
    }
}

/// `23.6`, `1200`, `-0.005`, `1.2e6`: short enough for labels.
pub fn number(v: f64) -> String {
    let abs = v.abs();
    if abs != 0.0 && !(0.001..1e6).contains(&abs) {
        return format!("{v:.2e}");
    }
    let text = format!("{v:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_owned()
    } else {
        text.to_owned()
    }
}

/// Draws the legend (first row), the value range on the left, and the
/// newest samples as points, one sample per column.
pub fn render(plot: &Plot, area: Rect, buf: &mut Buffer, color: bool) {
    if area.height < 3 || area.width < 12 {
        return;
    }
    let style_of = |c: Color| {
        if color {
            Style::default().fg(c)
        } else {
            Style::default()
        }
    };
    // Legend: shape, name, newest value.
    let mut x = area.x;
    for s in &plot.series {
        let latest = s.latest().map(number).unwrap_or_default();
        let text = format!("{} {} {latest}  ", s.marker, s.name);
        let width = u16::try_from(crate::term::screen::width(&text)).unwrap_or(u16::MAX);
        if x + width > area.right() {
            break;
        }
        buf.set_string(x, area.y, &text, style_of(s.color));
        x += width;
    }
    if plot.series.is_empty() {
        return;
    }
    let rows = area.height - 1;
    let top = area.y + 1;
    let labels = 8;
    let columns = usize::from(area.width.saturating_sub(labels + 1));
    let start = plot.series[0].values.len().saturating_sub(columns);
    let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
    for s in &plot.series {
        for v in s.values.iter().skip(start).flatten().copied() {
            low = low.min(v);
            high = high.max(v);
        }
    }
    if !low.is_finite() {
        return;
    }
    if (high - low).abs() < f64::EPSILON {
        low -= 1.0;
        high += 1.0;
    }
    let dim = Style::default().add_modifier(Modifier::DIM);
    let label = |v: f64| format!("{:>7}", crate::term::screen::truncate(&number(v), 7));
    buf.set_string(area.x, top, label(high), dim);
    buf.set_string(area.x, top + rows - 1, label(low), dim);
    for row in 0..rows {
        buf.set_string(area.x + labels - 1, top + row, "│", dim);
    }
    let span = high - low;
    for s in &plot.series {
        for (i, value) in s.values.iter().skip(start).enumerate() {
            let Some(v) = *value else { continue };
            let from_top = ((high - v) / span * f64::from(rows - 1)).round();
            #[allow(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "0..rows"
            )]
            let row = from_top.clamp(0.0, f64::from(rows - 1)) as u16;
            let Ok(col) = u16::try_from(i) else { break };
            let cell = &mut buf[(area.x + labels + col, top + row)];
            cell.set_char(s.marker);
            cell.set_style(style_of(s.color));
        }
    }
}

#[cfg(test)]
mod tests;
