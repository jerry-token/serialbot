use super::*;

fn names(plot: &Plot) -> Vec<&str> {
    plot.series.iter().map(|s| s.name.as_str()).collect()
}

#[test]
fn numbers_are_found_as_the_app_finds_them() {
    let sample = auto_sample("temp=23.6 hum: 41.2, x=1e3 bad=1.2.3 v2=-0.5");
    assert_eq!(
        sample,
        [
            ("temp".to_owned(), 23.6),
            ("hum".to_owned(), 41.2),
            ("x".to_owned(), 1000.0),
            ("v2".to_owned(), -0.5)
        ]
    );
    assert_eq!(
        auto_sample(" 1, 2.5;3 "),
        [
            ("ch1".to_owned(), 1.0),
            ("ch2".to_owned(), 2.5),
            ("ch3".to_owned(), 3.0)
        ]
    );
    assert!(auto_sample("boot ok 3 times").is_empty());
    assert!(auto_sample("").is_empty());
    assert_eq!(auto_sample("a=1 a=2"), [("a".to_owned(), 2.0)]);
}

#[test]
fn patterns_name_their_groups() {
    let p = Pattern::parse("temp=T:([0-9.]+)").unwrap();
    assert_eq!(p.names, ["temp"]);
    let p = Pattern::parse(r"acc ([-\d.]+),([-\d.]+),([-\d.]+)").unwrap();
    assert_eq!(p.names, ["acc", "acc2", "acc3"]);
    let p = Pattern::parse(r"(?P<x>\d+)/(?P<y>\d+)").unwrap();
    assert_eq!(p.names, ["x", "y"]);
    let p = Pattern::parse(r"\d+").unwrap();
    assert_eq!(p.names, ["value"]);
    assert!(Pattern::parse("t=(").unwrap_err().contains("--plot"));
    // `=` inside a regex is not a name.
    let p = Pattern::parse(r"a\s*=\s*(\d+)").unwrap();
    assert_eq!(p.names, ["a"]);

    let mut plot = Plot::new(vec![
        Pattern::parse("temp=T:([0-9.]+)").unwrap(),
        Pattern::parse(r"(?P<x>\d+)/(?P<y>\d+)").unwrap(),
    ]);
    plot.push(b"T:21.5 at 3/4\r\nnothing\nT:22\n");
    assert_eq!(names(&plot), ["temp", "x", "y"]);
    assert_eq!(plot.samples, 2, "lines without numbers are no sample");
    assert_eq!(plot.series[1].values, [Some(3.0), None]);
    assert_eq!(plot.series[0].latest(), Some(22.0));
}

#[test]
fn curves_get_colors_and_shapes_in_order_and_at_most_six() {
    let mut plot = Plot::default();
    plot.push(b"a=1 b=2\n");
    plot.push(b"\x1b[32mc=3\x1b[0m d=4 e=5 f=6 g=7\n");
    assert_eq!(names(&plot), ["a", "b", "c", "d", "e", "f"]);
    let shapes: Vec<char> = plot.series.iter().map(|s| s.marker).collect();
    assert_eq!(shapes, ['•', '×', '+', '▪', '◆', '○']);
    assert!(
        !plot.series.iter().any(|s| s.color == Color::Red),
        "red is for errors"
    );
    // A curve that appears later has gaps before it.
    assert_eq!(plot.series[2].values, [None, Some(3.0)]);
    assert_eq!(plot.series[0].values, [Some(1.0), None]);
    plot.set_color("c", "Yellow").unwrap();
    assert_eq!(plot.series[2].color, Color::Yellow);
    assert!(plot.set_color("c", "pink").unwrap_err().contains("cyan"));
    assert!(plot.set_color("zz", "red").unwrap_err().contains("a, b"));
    plot.clear();
    assert!(plot.series.is_empty());
    assert_eq!(plot.samples, 0);
}

#[test]
fn keeps_the_newest_samples() {
    let mut plot = Plot::default();
    for i in 0..CAPACITY + 10 {
        plot.line(&format!("v={i}"));
    }
    assert_eq!(plot.series[0].values.len(), CAPACITY);
    assert_eq!(plot.series[0].values[0], Some(10.0));
    plot.line("w=1");
    assert_eq!(plot.series[1].values.len(), CAPACITY);
}

#[test]
fn labels_are_short() {
    assert_eq!(number(23.6), "23.6");
    assert_eq!(number(1200.0), "1200");
    assert_eq!(number(-0.005), "-0.005");
    assert_eq!(number(0.0), "0");
    assert_eq!(number(-0.0001), "-1.00e-4");
    assert_eq!(number(2.5e7), "2.50e7");
}

fn rendered(plot: &Plot, color: bool) -> Buffer {
    let area = Rect::new(0, 0, 40, 8);
    let mut buf = Buffer::empty(area);
    render(plot, area, &mut buf, color);
    buf
}

fn rows(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

#[test]
fn draws_a_legend_the_range_and_points() {
    let mut plot = Plot::default();
    for i in 0..10 {
        plot.line(&format!("up={i} down={}", 9 - i));
    }
    let buf = rendered(&plot, true);
    let rows = rows(&buf);
    assert!(rows[0].starts_with("• up 9  × down 0"), "{rows:?}");
    assert!(rows[1].starts_with("      9│"), "{rows:?}");
    assert!(rows[7].starts_with("      0│"), "{rows:?}");
    // `up` starts at the bottom and ends at the top, `down` the other way.
    assert_eq!(buf[(8, 7)].symbol(), "•");
    assert_eq!(buf[(8, 7)].fg, Color::Cyan);
    assert_eq!(buf[(8, 1)].symbol(), "×");
    assert_eq!(buf[(8, 1)].fg, Color::Yellow);
    assert_eq!(buf[(17, 1)].symbol(), "•");
    // Without color the shapes still differ.
    let plain = rendered(&plot, false);
    assert_eq!(plain[(8, 1)].symbol(), "×");
    assert_eq!(plain[(8, 1)].fg, Color::Reset);
}

#[test]
fn flat_empty_and_tiny_plots() {
    let mut plot = Plot::default();
    assert!(
        rows(&rendered(&plot, true))
            .iter()
            .all(|r| r.trim().is_empty())
    );
    plot.line("flat=5");
    plot.line("flat=5");
    let rows = rows(&rendered(&plot, true));
    assert!(
        rows[1].starts_with("      6│"),
        "a flat line gets a range: {rows:?}"
    );
    let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
    render(&plot, buf.area, &mut buf, true);
    assert!(buf.content.iter().all(|c| c.symbol() == " "));
}
