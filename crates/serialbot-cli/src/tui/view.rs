//! Draws [`Tui`] into a ratatui frame: the session list on the left (on
//! wide screens), the shown sessions side by side or stacked, the input
//! box and the status bar.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::app::{Pane, Regions, Tui, WIDE};
use super::sgr;
use crate::term::app::{input_lines, prefix_hint};
use crate::term::screen::truncate;

/// Width of the session list.
const LIST_WIDTH: u16 = 24;

/// At most this many input rows are shown.
const INPUT_ROWS: usize = 3;

/// Draws everything and remembers where for the mouse.
pub fn render(tui: &mut Tui, frame: &mut Frame<'_>) {
    let area = frame.area();
    let (input, cursor) = input_lines(&tui.keys);
    let input_rows = input.len().clamp(1, INPUT_ROWS);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(u16::try_from(input_rows).unwrap_or(1)),
            Constraint::Length(1),
        ])
        .split(area);
    let (main, input_area, status_area) = (rows[0], rows[1], rows[2]);

    tui.wide = area.width >= WIDE;
    let mut regions = Regions::default();
    let panes_area = if tui.wide && !tui.panes.is_empty() {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(LIST_WIDTH), Constraint::Min(1)])
            .split(main);
        draw_list(tui, frame, cols[0]);
        regions.list = cols[0];
        cols[1]
    } else {
        main
    };

    if tui.panes.is_empty() {
        draw_start(tui, frame, panes_area);
    } else {
        let shown: Vec<usize> = if tui.wide {
            tui.shown.clone()
        } else {
            vec![tui.focus]
        };
        let direction = if tui.side_by_side {
            Direction::Horizontal
        } else {
            Direction::Vertical
        };
        let count = u32::try_from(shown.len()).unwrap_or(1);
        let areas = Layout::default()
            .direction(direction)
            .constraints(shown.iter().map(|_| Constraint::Ratio(1, count)))
            .split(panes_area);
        for (i, area) in shown.iter().zip(areas.iter()) {
            let focused = *i == tui.focus;
            draw_pane(&tui.panes[*i], focused, tui, frame, *area);
            regions.panes.push((*i, *area));
        }
    }
    tui.regions = regions;

    draw_input(tui, frame, input_area, &input, cursor);
    draw_status(tui, frame, status_area);
}

fn draw_start(tui: &Tui, frame: &mut Frame<'_>, area: Rect) {
    let t = tui.texts();
    let mut lines = vec![Line::from(t.start)];
    lines.extend(
        tui.messages
            .iter()
            .map(|m| Line::from(Span::styled(format!("· {m}"), dim()))),
    );
    let block = Block::default().borders(Borders::ALL).title(" SerialBot ");
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_list(tui: &Tui, frame: &mut Frame<'_>, area: Rect) {
    let width = usize::from(area.width.saturating_sub(2));
    let lines: Vec<Line<'_>> = tui
        .panes
        .iter()
        .enumerate()
        .map(|(i, pane)| {
            let dot = if pane.unread { " •" } else { "" };
            let name = tail(&pane.name, width.saturating_sub(2 + dot.chars().count()));
            let text = format!("{} {name}{dot}", pane.state_mark());
            let style = if i == tui.focus {
                Style::default().add_modifier(Modifier::REVERSED)
            } else if tui.shown.contains(&i) {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            Line::from(Span::styled(text, style))
        })
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", tui.texts().sessions));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// The rows of `pane` that fit `height` rows of `width`, scrolled back
/// by `pane.scroll` (clamped to what there is). The unfinished line
/// comes last.
pub fn pane_rows(
    pane: &Pane,
    width: usize,
    height: usize,
    color: bool,
) -> (Vec<Line<'static>>, usize) {
    let wrap = pane.console.wrap;
    let pending = pane.console.feed.pending();
    let wanted = height + pane.scroll;
    let mut rows: Vec<Line<'static>> = Vec::new();
    let all = pane
        .lines
        .iter()
        .map(String::as_str)
        .chain(pending.as_deref());
    let lines: Vec<&str> = all.collect();
    for line in lines.iter().rev() {
        let parsed = sgr::line(line, color);
        let mut wrapped = if wrap {
            sgr::wrap(&parsed, width)
        } else {
            vec![sgr::cut(&parsed, width)]
        };
        wrapped.reverse();
        rows.extend(wrapped);
        if rows.len() >= wanted {
            break;
        }
    }
    let scroll = pane.scroll.min(rows.len().saturating_sub(height));
    let visible: Vec<Line<'static>> = rows.into_iter().skip(scroll).take(height).rev().collect();
    (visible, scroll)
}

/// Rows the waveform gets under a pane's lines: half, at least
/// [`PLOT_MIN`] when the pane is tall enough.
const PLOT_MIN: u16 = 6;

fn draw_pane(pane: &Pane, focused: bool, tui: &Tui, frame: &mut Frame<'_>, area: Rect) {
    let color = tui.color();
    let inner = area.inner(ratatui::layout::Margin::new(1, 1));
    let (text_area, plot_area) = if pane.plot_shown && inner.height > PLOT_MIN {
        let plot_rows = (inner.height / 2).max(PLOT_MIN);
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(plot_rows)])
            .split(inner);
        (split[0], Some(split[1]))
    } else {
        (inner, None)
    };
    let inner_width = usize::from(text_area.width);
    let inner_height = usize::from(text_area.height);
    let (rows, scroll) = pane_rows(pane, inner_width, inner_height, color);
    let mark = if focused { "▸ " } else { "" };
    let back = if scroll > 0 {
        format!(" ↑{scroll}")
    } else {
        String::new()
    };
    let room = inner_width.saturating_sub(8 + back.chars().count());
    let title = format!(
        " {mark}{} {}{back} ",
        pane.state_mark(),
        tail(&pane.name, room)
    );
    let border = match (focused, color) {
        (true, true) => Style::default().fg(Color::Cyan),
        (true, false) => Style::default().add_modifier(Modifier::BOLD),
        (false, _) => dim(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .title(Span::styled(title, border));
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(rows), text_area);
    if let Some(plot_area) = plot_area {
        let rule = "─".repeat(usize::from(plot_area.width));
        frame
            .buffer_mut()
            .set_string(plot_area.x, plot_area.y, rule, dim());
        let chart = Rect {
            y: plot_area.y + 1,
            height: plot_area.height - 1,
            ..plot_area
        };
        if pane.plot.series.is_empty() {
            frame.buffer_mut().set_stringn(
                chart.x,
                chart.y,
                tui.texts().plot_empty,
                usize::from(chart.width),
                dim(),
            );
        } else {
            super::plot::render(&pane.plot, chart, frame.buffer_mut(), color);
        }
    }
}

fn draw_input(
    tui: &Tui,
    frame: &mut Frame<'_>,
    area: Rect,
    input: &[String],
    cursor: (usize, usize),
) {
    let prompt = if tui.keys.search.is_some() {
        "(search) "
    } else {
        "› "
    };
    // Keep the cursor's row in view when there are more rows than fit.
    let first = cursor.0.saturating_sub(INPUT_ROWS - 1);
    let lines: Vec<Line<'_>> = input
        .iter()
        .enumerate()
        .skip(first)
        .take(INPUT_ROWS)
        .map(|(i, text)| {
            let lead = if i == 0 { prompt } else { "  " };
            Line::from(format!("{lead}{text}"))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
    let lead = u16::try_from(crate::term::screen::width(prompt)).unwrap_or(2);
    let column: String = input
        .get(cursor.0)
        .map(|l| l.chars().take(cursor.1).collect())
        .unwrap_or_default();
    let x = area.x + lead + u16::try_from(crate::term::screen::width(&column)).unwrap_or(0);
    let y = area.y + u16::try_from(cursor.0 - first).unwrap_or(0);
    frame.set_cursor_position(Position::new(x.min(area.right().saturating_sub(1)), y));
}

fn draw_status(tui: &Tui, frame: &mut Frame<'_>, area: Rect) {
    let term = tui.lang().strings().term;
    let mut parts = tui
        .focused()
        .map(|p| p.console.status_parts())
        .unwrap_or_default();
    let others = tui.panes.iter().filter(|p| p.unread).count();
    if others > 0 {
        parts.push(format!("• {others}"));
    }
    parts.push(prefix_hint(&tui.keys, term));
    let text = truncate(&parts.join("  "), usize::from(area.width));
    frame.render_widget(Paragraph::new(Line::from(Span::styled(text, dim()))), area);
}

/// `text` in at most `max` columns, its end kept (`…/by-id/usb-FTDI`):
/// the end of a port path tells ports apart.
pub fn tail(text: &str, max: usize) -> String {
    if crate::term::screen::width(text) <= max {
        return text.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut kept: Vec<char> = Vec::new();
    let mut used = 1;
    for c in text.chars().rev() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > max {
            break;
        }
        used += w;
        kept.push(c);
    }
    kept.reverse();
    format!("…{}", kept.into_iter().collect::<String>())
}

fn dim() -> Style {
    Style::default().add_modifier(Modifier::DIM)
}

#[cfg(test)]
mod tests {
    use super::tail;

    #[test]
    fn long_names_keep_their_end() {
        assert_eq!(tail("/dev/ttyUSB0", 20), "/dev/ttyUSB0");
        assert_eq!(tail("/dev/serial/by-id/usb-FTDI-if00", 10), "…FTDI-if00");
        assert_eq!(tail("串口一号", 5), "…一号");
        assert_eq!(tail("abc", 0), "");
    }
}
