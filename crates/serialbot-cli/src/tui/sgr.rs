//! Lines with color sequences (`ESC [ … m`, as the console makes them and
//! devices send them) to styled ratatui text, and wrapping that text to a
//! width.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthChar;

/// Parses the color sequences in `text`; any other escape sequence is
/// dropped. With `color` off, only the text is kept.
pub fn line(text: &str, color: bool) -> Line<'static> {
    let mut spans = Vec::new();
    let mut style = Style::default();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            current.push(c);
            continue;
        }
        if chars.peek() != Some(&'[') {
            continue;
        }
        chars.next();
        let mut params = String::new();
        let mut last = None;
        for c in chars.by_ref() {
            if ('\x40'..='\x7e').contains(&c) {
                last = Some(c);
                break;
            }
            params.push(c);
        }
        if last != Some('m') {
            continue;
        }
        if !current.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut current), style));
        }
        if color {
            style = apply(style, &params);
        }
    }
    if !current.is_empty() {
        spans.push(Span::styled(current, style));
    }
    Line::from(spans)
}

/// `style` after the SGR parameters `params` (`1;36`).
fn apply(mut style: Style, params: &str) -> Style {
    let codes: Vec<u16> = if params.is_empty() {
        vec![0]
    } else {
        params.split(';').map(|p| p.parse().unwrap_or(0)).collect()
    };
    let mut i = 0;
    while i < codes.len() {
        let code = codes[i];
        style = match code {
            0 => Style::default(),
            1 => style.add_modifier(Modifier::BOLD),
            2 => style.add_modifier(Modifier::DIM),
            3 => style.add_modifier(Modifier::ITALIC),
            4 => style.add_modifier(Modifier::UNDERLINED),
            7 => style.add_modifier(Modifier::REVERSED),
            22 => style.remove_modifier(Modifier::BOLD | Modifier::DIM),
            23 => style.remove_modifier(Modifier::ITALIC),
            24 => style.remove_modifier(Modifier::UNDERLINED),
            27 => style.remove_modifier(Modifier::REVERSED),
            30..=37 => style.fg(basic(code - 30, false)),
            39 => style.fg(Color::Reset),
            40..=47 => style.bg(basic(code - 40, false)),
            49 => style.bg(Color::Reset),
            90..=97 => style.fg(basic(code - 90, true)),
            100..=107 => style.bg(basic(code - 100, true)),
            38 | 48 => {
                // 256 colors (`38;5;n`) or true color (`38;2;r;g;b`).
                let (color, used) = match codes.get(i + 1) {
                    Some(5) => (
                        codes
                            .get(i + 2)
                            .and_then(|n| u8::try_from(*n).ok())
                            .map(Color::Indexed),
                        2,
                    ),
                    Some(2) => {
                        let part = |k: usize| codes.get(i + k).and_then(|n| u8::try_from(*n).ok());
                        (
                            part(2)
                                .zip(part(3))
                                .zip(part(4))
                                .map(|((r, g), b)| Color::Rgb(r, g, b)),
                            4,
                        )
                    }
                    _ => (None, 0),
                };
                i += used;
                match (color, code) {
                    (Some(c), 38) => style.fg(c),
                    (Some(c), _) => style.bg(c),
                    (None, _) => style,
                }
            }
            _ => style,
        };
        i += 1;
    }
    style
}

/// One of the eight terminal colors, or its bright form.
fn basic(n: u16, bright: bool) -> Color {
    match (n, bright) {
        (0, false) => Color::Black,
        (1, false) => Color::Red,
        (2, false) => Color::Green,
        (3, false) => Color::Yellow,
        (4, false) => Color::Blue,
        (5, false) => Color::Magenta,
        (6, false) => Color::Cyan,
        (7, false) => Color::Gray,
        (0, true) => Color::DarkGray,
        (1, true) => Color::LightRed,
        (2, true) => Color::LightGreen,
        (3, true) => Color::LightYellow,
        (4, true) => Color::LightBlue,
        (5, true) => Color::LightMagenta,
        (6, true) => Color::LightCyan,
        _ => Color::White,
    }
}

/// `line` cut into rows at most `width` columns wide (at least one row,
/// even for an empty line). A wide character never straddles two rows.
pub fn wrap(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut row: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for span in &line.spans {
        let mut piece = String::new();
        for c in span.content.chars() {
            let w = c.width().unwrap_or(0);
            if used + w > width && used > 0 {
                if !piece.is_empty() {
                    row.push(Span::styled(std::mem::take(&mut piece), span.style));
                }
                rows.push(Line::from(std::mem::take(&mut row)));
                used = 0;
            }
            piece.push(c);
            used += w;
        }
        if !piece.is_empty() {
            row.push(Span::styled(piece, span.style));
        }
    }
    rows.push(Line::from(row));
    rows
}

/// `line` cut at `width` columns (no wrapping, `Ctrl+A W`).
pub fn cut(line: &Line<'static>, width: usize) -> Line<'static> {
    wrap(line, width).swap_remove(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn colors_become_styles_and_other_sequences_go() {
        let l = line("\x1b[2m[12:00]\x1b[0m \x1b[1;36mok\x1b[0m\x1b[2K end", true);
        assert_eq!(text(&l), "[12:00] ok end");
        assert!(l.spans[0].style.add_modifier.contains(Modifier::DIM));
        assert_eq!(l.spans[2].style.fg, Some(Color::Cyan));
        assert!(l.spans[2].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(l.spans[3].style, Style::default());
        let plain = line("\x1b[31merr\x1b[m", false);
        assert_eq!(text(&plain), "err");
        assert_eq!(plain.spans[0].style, Style::default());
    }

    #[test]
    fn every_sgr_form() {
        let style = |p: &str| apply(Style::default(), p);
        assert_eq!(style("91").fg, Some(Color::LightRed));
        assert_eq!(style("44").bg, Some(Color::Blue));
        assert_eq!(style("103").bg, Some(Color::LightYellow));
        assert_eq!(style("38;5;208").fg, Some(Color::Indexed(208)));
        assert_eq!(style("48;2;1;2;3").bg, Some(Color::Rgb(1, 2, 3)));
        assert_eq!(style("38;9").fg, None);
        assert_eq!(style("31;39").fg, Some(Color::Reset));
        assert_eq!(style("41;49").bg, Some(Color::Reset));
        let all = style("1;3;4;7");
        let undone = apply(all, "22;23;24;27");
        assert_eq!(undone.add_modifier, Modifier::empty());
        assert_eq!(style("").fg, None);
        for n in 0..8 {
            assert_ne!(basic(n, false), basic(n, true));
        }
        // A lone escape and an unfinished sequence are dropped.
        assert_eq!(text(&line("a\x1bb\x1b[12", true)), "ab");
    }

    #[test]
    fn wraps_by_columns_and_keeps_styles() {
        let l = line("ab\x1b[31mcdef\x1b[0m", true);
        let rows = wrap(&l, 3);
        let texts: Vec<String> = rows.iter().map(text).collect();
        assert_eq!(texts, ["abc", "def"]);
        assert_eq!(rows[1].spans[0].style.fg, Some(Color::Red));
        let wide: Vec<String> = wrap(&line("中文字", true), 5).iter().map(text).collect();
        assert_eq!(wide, ["中文", "字"]);
        assert_eq!(wrap(&line("", true), 4).len(), 1);
        assert_eq!(text(&cut(&line("abcdef", true), 4)), "abcd");
        assert_eq!(wrap(&line("x", true), 0).len(), 1);
    }
}
