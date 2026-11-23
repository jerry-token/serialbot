// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serialbot_core::transport::mock::MockDevice;

use super::*;
use crate::args::{Cli, Command};
use crate::commands::tui::{Env, prepare};
use crate::testing::Fixture;
use crate::tui::ai::Ai;
use crate::tui::view;

fn env(f: &Fixture, color: bool, lang: Lang) -> Env {
    let ports = f.ports.clone();
    Env {
        terminal: true,
        interrupted: Arc::default(),
        utc_offset_min: 0,
        color,
        lang,
        list_ports: Arc::new(move || Ok(ports.clone())),
        config_dir: Some(f.config_dir.clone()),
    }
}

/// `tui <args>` as parsed from a command line.
fn start(f: &Fixture, args: &[&str], color: bool, lang: Lang) -> Tui {
    let mut line = vec!["serialbot-cli", "tui"];
    line.extend_from_slice(args);
    let Command::Tui(args) = Cli::try_parse_from(line).unwrap().command else {
        panic!("not tui");
    };
    prepare(&args, &f.manager, &env(f, color, lang)).unwrap()
}

struct Rig {
    f: Fixture,
    tui: Tui,
    term: Terminal<TestBackend>,
}

impl Rig {
    fn new(args: &[&str], width: u16, height: u16) -> Self {
        let f = Fixture::new();
        let tui = start(&f, args, false, Lang::En);
        let term = Terminal::new(TestBackend::new(width, height)).unwrap();
        let mut rig = Self { f, tui, term };
        rig.draw();
        rig
    }

    fn device(&self, port: &str) -> MockDevice {
        self.f.devices.lock().unwrap()[port].clone()
    }

    fn draw(&mut self) {
        let tui = &mut self.tui;
        self.term.draw(|frame| view::render(tui, frame)).unwrap();
    }

    fn screen(&self) -> String {
        let buffer = self.term.backend().buffer();
        let mut text = String::new();
        for y in 0..buffer.area.height {
            let mut skip = 0;
            for x in 0..buffer.area.width {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                let symbol = buffer[(x, y)].symbol();
                skip = unicode_width::UnicodeWidthStr::width(symbol).saturating_sub(1);
                text.push_str(symbol);
            }
            text.push('\n');
        }
        text
    }

    /// Whether a line of session `i` contains `text`.
    fn logged(&self, i: usize, text: &str) -> bool {
        self.tui.panes[i].lines.iter().any(|l| l.contains(text))
    }

    fn row(&self, y: u16) -> String {
        self.screen()
            .lines()
            .nth(usize::from(y))
            .unwrap_or_default()
            .to_owned()
    }

    fn keys(&mut self, keys: &[KeyEvent]) -> Flow {
        let mut flow = Flow::Continue;
        for key in keys {
            flow = self.tui.key(*key, 30);
        }
        self.draw();
        flow
    }

    fn type_line(&mut self, text: &str) -> Flow {
        let mut keys: Vec<KeyEvent> = text.chars().map(|c| press(KeyCode::Char(c))).collect();
        keys.push(press(KeyCode::Enter));
        self.keys(&keys)
    }

    fn prefix(&mut self, code: KeyCode) -> Flow {
        self.keys(&[ctrl('a'), press(code)])
    }

    /// Ticks and draws until the screen shows `text` (five seconds at most).
    fn until_shows(&mut self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            self.tui.tick();
            self.draw();
            if self.screen().contains(text) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "no {text:?} in\n{}",
                self.screen()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn click(&mut self, column: u16, row: u16) {
        self.mouse(MouseEventKind::Down(MouseButton::Left), column, row);
    }

    fn mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) {
        self.tui.mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
        self.draw();
    }
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

#[test]
fn starts_empty_and_opens_sessions_from_the_input_box() {
    let mut r = Rig::new(&[], 120, 30);
    assert!(
        r.screen().contains("No session yet: type /open"),
        "{}",
        r.screen()
    );
    r.type_line("hello");
    assert!(
        r.screen().contains("· no session is open"),
        "{}",
        r.screen()
    );
    r.type_line("/open mock0 9600");
    assert_eq!(r.tui.panes.len(), 1);
    assert!(r.screen().contains("▸ ● mock0"), "{}", r.screen());
    assert!(
        r.screen().contains("Sessions"),
        "the list shows on wide screens"
    );
    assert!(
        r.row(29).contains("mock0  9600 8N1  RX 0 TX 0"),
        "{}",
        r.row(29)
    );
    r.type_line("AT");
    assert_eq!(r.device("mock0").take_written(), b"AT\r\n");
    r.until_shows("→ AT\\r\\n");
    r.device("mock0").send(b"OK\r\n");
    r.until_shows("│OK");
    // Opening it again focuses it instead.
    r.type_line("/open mock0");
    assert_eq!(r.tui.panes.len(), 1);
    r.type_line("/open busy0");
    assert!(r.screen().contains("cannot open busy0"), "{}", r.screen());
    r.type_line("/open mock0 fast");
    assert!(
        r.screen().contains("/open mock0 fast: unknown command"),
        "{}",
        r.screen()
    );
}

#[test]
fn panes_side_by_side_or_stacked_and_the_focus_moves() {
    let mut r = Rig::new(&["mock0", "mock1", "mock2"], 120, 30);
    assert_eq!(r.tui.shown, [0, 1, 2]);
    assert_eq!(r.tui.focus, 0, "the first port has the focus");
    let top = r.row(0);
    assert!(
        top.contains("▸ ● mock0") && top.contains("● mock1") && top.contains("● mock2"),
        "{top}"
    );
    r.prefix(KeyCode::Right);
    assert_eq!(r.tui.focus, 1);
    r.prefix(KeyCode::Left);
    r.prefix(KeyCode::Left);
    assert_eq!(r.tui.focus, 2, "it wraps around");
    r.type_line("to two");
    assert_eq!(r.device("mock2").take_written(), b"to two\r\n");
    assert!(r.device("mock0").take_written().is_empty());

    r.prefix(KeyCode::Char('-'));
    assert!(!r.tui.side_by_side);
    let rows = r.screen();
    let at = |name: &str| rows.lines().position(|l| l.contains(name)).unwrap();
    assert!(
        at("● mock0") < at("● mock1") && at("● mock1") < at("● mock2"),
        "{rows}"
    );
    r.prefix(KeyCode::Char('|'));
    assert!(r.tui.side_by_side);

    // A fifth session replaces the focused pane; four fit.
    r.type_line("/open mock3");
    assert_eq!(r.tui.shown, [0, 1, 2, 3]);
    r.type_line("/open mock4");
    assert_eq!(r.tui.shown, [0, 1, 2, 4]);
    assert_eq!(r.tui.focus, 4);
}

#[test]
fn unread_dots_clicks_and_the_list() {
    let mut r = Rig::new(&["mock0", "mock1"], 120, 30);
    r.tui.select(0);
    // Only one shown: mock1 goes out of view.
    r.tui.shown = vec![0];
    r.draw();
    r.device("mock1").send(b"news\n");
    r.until_shows("● mock1 •");
    assert!(r.row(29).contains("• 1"), "{}", r.row(29));
    // Click its row in the list (rows start under the border).
    let list = r.tui.regions.list;
    r.click(list.x + 2, list.y + 2);
    assert_eq!(r.tui.focus, 1);
    assert_eq!(
        r.tui.shown,
        [0, 1],
        "shown in a new pane while there is room"
    );
    assert!(!r.tui.panes[1].unread);
    r.until_shows("news");
    // A click in a pane focuses it.
    let (_, area) = r.tui.regions.panes[0];
    r.click(area.x + 3, area.y + 3);
    assert_eq!(r.tui.focus, 0);
    // The splitting keys bring a hidden session in.
    r.tui.shown = vec![0];
    r.prefix(KeyCode::Char('|'));
    assert_eq!(r.tui.shown, [0, 1]);
}

#[test]
fn scrolls_back_with_keys_and_the_wheel() {
    let mut r = Rig::new(&["mock0"], 120, 30);
    let lines = (0..100)
        .map(|i| format!("line {i}\n"))
        .collect::<Vec<_>>()
        .concat();
    r.device("mock0").send(lines.as_bytes());
    r.until_shows("line 99");
    r.keys(&[press(KeyCode::PageUp)]);
    assert_eq!(r.tui.panes[0].scroll, 15);
    assert!(!r.screen().contains("line 99"));
    assert!(r.screen().contains("↑15"), "{}", r.screen());
    // New data does not move what is being read.
    r.device("mock0").send(b"line 100\n");
    r.until_shows("↑16");
    let (_, area) = r.tui.regions.panes[0];
    r.mouse(MouseEventKind::ScrollDown, area.x + 2, area.y + 2);
    assert_eq!(r.tui.panes[0].scroll, 13);
    r.keys(&[press(KeyCode::PageDown)]);
    assert_eq!(r.tui.panes[0].scroll, 0);
    r.mouse(MouseEventKind::ScrollUp, area.x + 2, area.y + 2);
    assert_eq!(r.tui.panes[0].scroll, 3);
    // Sending follows the newest data again.
    r.type_line("x");
    assert_eq!(r.tui.panes[0].scroll, 0);
    // Scrolling far back stops at the first line.
    r.tui.panes[0].scroll = 10_000;
    r.draw();
    assert!(r.screen().contains("line 0"), "{}", r.screen());
}

#[test]
fn narrow_screens_show_only_the_focused_session() {
    let mut r = Rig::new(&["mock0", "mock1"], 80, 20);
    assert!(!r.tui.wide);
    assert!(!r.screen().contains("Sessions"));
    assert_eq!(r.tui.regions.panes.len(), 1);
    assert!(r.row(0).contains("▸ ● mock0"), "{}", r.row(0));
    r.prefix(KeyCode::Right);
    assert!(r.row(0).contains("▸ ● mock1"), "{}", r.row(0));
}

#[test]
fn console_commands_reach_the_focused_session() {
    let mut r = Rig::new(&["mock0", "mock1"], 120, 30);
    r.tui.select(1);
    r.type_line("/hex");
    assert!(r.tui.panes[1].console.feed.display.hex);
    assert!(!r.tui.panes[0].console.feed.display.hex);
    r.device("mock1").send(b"AB");
    r.until_shows("41 42");
    r.prefix(KeyCode::Char('c'));
    assert!(r.tui.panes[1].lines.is_empty());
    r.prefix(KeyCode::Char('z'));
    assert!(r.logged(1, "Ctrl+A ←/→ focus"));
    assert!(r.logged(1, "Ctrl+A then: Z help"), "the keys of open too");
    r.type_line("/raw");
    assert!(r.logged(1, "raw mode works in serialbot-cli open"));
    r.prefix(KeyCode::Char('j'));
    assert!(r.logged(1, "Ctrl+A J works in serialbot-cli open"));
    r.prefix(KeyCode::Char('g'));
    assert_eq!(r.tui.keys.editor.text(), "/run ");
    r.keys(&[press(KeyCode::Esc)]);
    r.keys(&[ctrl('a'), ctrl('a')]);
    assert_eq!(r.device("mock1").take_written(), [0x01]);
    r.keys(&[ctrl('a')]);
    assert!(r.row(29).contains("Ctrl+A …"), "{}", r.row(29));
    r.keys(&[press(KeyCode::Esc)]);
    r.tui.paste("pasted");
    assert_eq!(r.tui.keys.editor.text(), "pasted");
    r.keys(&[press(KeyCode::Esc)]);

    r.type_line("/close");
    assert!(r.screen().contains("closed mock1"), "{}", r.screen());
    for _ in 0..3 {
        r.tui.tick();
    }
    r.draw();
    assert!(r.screen().contains("○ mock1"), "the session stays, closed");
    assert!(!r.logged(1, "waiting"), "a closed port is not waited for");
    assert_eq!(r.type_line("/quit"), Flow::Quit);
    assert_eq!(r.prefix(KeyCode::Char('x')), Flow::Quit);
    r.tui.close_all();
    assert!(r.f.manager.list().is_empty());
}

#[test]
fn colors_follow_the_device_and_no_color_keeps_text() {
    let f = Fixture::new();
    let mut tui = start(&f, &["mock0"], true, Lang::Zh);
    let mut term = Terminal::new(TestBackend::new(120, 20)).unwrap();
    f.devices.lock().unwrap()["mock0"].send(b"\x1b[32mgreen\x1b[0m\n");
    let deadline = Instant::now() + Duration::from_secs(5);
    while tui.panes[0].lines.is_empty() {
        assert!(Instant::now() < deadline);
        tui.tick();
        std::thread::sleep(Duration::from_millis(5));
    }
    term.draw(|frame| view::render(&mut tui, frame)).unwrap();
    let buffer = term.backend().buffer();
    let (_, area) = tui.regions.panes[0];
    let cell = &buffer[(area.x + 1, area.y + 1)];
    assert_eq!(cell.symbol(), "g");
    assert_eq!(cell.fg, ratatui::style::Color::Green);
    assert_eq!(
        buffer[(area.x, area.y)].fg,
        ratatui::style::Color::Cyan,
        "focused border"
    );
    let text: String = (0..120)
        .map(|x| buffer[(x, 0)].symbol().to_owned())
        .collect();
    assert!(text.contains('会') && text.contains('话'), "{text}");
    tui.close_all();
}

#[test]
fn bad_options_and_ports_fail_before_the_screen() {
    let f = Fixture::new();
    let parse = |extra: &[&str]| {
        let mut argv = vec!["serialbot-cli", "tui"];
        argv.extend_from_slice(extra);
        match Cli::try_parse_from(argv).unwrap().command {
            Command::Tui(a) => a,
            _ => panic!("not tui"),
        }
    };
    let e = env(&f, false, Lang::En);
    let fail = |extra: &[&str]| prepare(&parse(extra), &f.manager, &e).unwrap_err();
    assert_eq!(fail(&["--encoding", "nope"]).exit, crate::exit::Exit::Usage);
    assert_eq!(
        fail(&["--reset-sequence", "dtr=x"]).exit,
        crate::exit::Exit::Usage
    );
    assert_eq!(fail(&["mock0", "busy0"]).exit, crate::exit::Exit::Busy);
    assert!(
        f.manager.list().is_empty(),
        "the ports opened before are closed"
    );
    let ran = f.run(&["tui", "mock0"]);
    assert_eq!(ran.code, 2, "no terminal");
    assert!(ran.err.contains("tui needs a terminal"), "{}", ran.err);
    let tui = prepare(
        &parse(&["--hex", "--time", "-b", "9600", "mock0"]),
        &f.manager,
        &e,
    )
    .unwrap();
    let pane = &tui.panes[0];
    assert!(pane.console.feed.display.hex);
    assert_eq!(pane.console.feed.display.stamps, Stamps::Time);
    assert_eq!(pane.console.settings().baud, 9600);
}

#[test]
fn the_waveform_per_session() {
    let mut r = Rig::new(&["mock0", "mock1"], 120, 30);
    r.prefix(KeyCode::Char('v'));
    assert!(r.tui.panes[0].plot_shown && !r.tui.panes[1].plot_shown);
    assert!(r.screen().contains("waiting for numbers"), "{}", r.screen());
    r.device("mock0").send(b"temp=20 hum=40\ntemp=21 hum=41\n");
    r.until_shows("• temp 21");
    assert!(r.screen().contains("× hum 41"), "{}", r.screen());
    // The lines still show above it.
    assert!(r.screen().contains("temp=21 hum=41"));

    r.type_line("/plot color hum green");
    assert_eq!(
        r.tui.panes[0].plot.series[1].color,
        ratatui::style::Color::Green
    );
    r.type_line("/plot color hum pink");
    assert!(
        r.logged(0, "unknown; choose one of: pink: cyan"),
        "{:?}",
        r.tui.panes[0].lines
    );
    r.type_line("/plot t=temp=(\\d+)");
    assert_eq!(r.tui.panes[0].plot.patterns.len(), 1);
    r.device("mock0").send(b"temp=30 hum=50\n");
    r.until_shows("• t 30");
    assert!(!r.screen().contains("hum 50"));
    r.type_line("/plot (");
    assert!(r.logged(0, "--plot \"(\""), "{:?}", r.tui.panes[0].lines);
    r.type_line("/plot clear");
    assert!(r.tui.panes[0].plot.series.is_empty());
    r.type_line("/plot auto");
    assert!(r.tui.panes[0].plot.patterns.is_empty());
    r.type_line("/plot");
    assert!(!r.tui.panes[0].plot_shown);
    // Numbers are taken while it is hidden, too.
    r.device("mock0").send(b"1,2\n");
    r.until_shows("1,2");
    assert_eq!(r.tui.panes[0].plot.series.len(), 2);
}

#[test]
fn plot_patterns_from_the_command_line() {
    let f = Fixture::new();
    let tui = start(
        &f,
        &["--plot", "t=T(\\d+)", "--plot", "h=H(\\d+)", "mock0"],
        false,
        Lang::En,
    );
    assert!(tui.panes[0].plot_shown);
    assert_eq!(tui.panes[0].plot.patterns.len(), 2);
    let line = vec!["serialbot-cli", "tui", "--plot", "(", "mock1"];
    let Command::Tui(args) = Cli::try_parse_from(line).unwrap().command else {
        panic!("not tui");
    };
    let e = prepare(&args, &f.manager, &env(&f, false, Lang::En)).unwrap_err();
    assert_eq!(e.exit, crate::exit::Exit::Usage);
    let mut empty = Rig::new(&[], 120, 30);
    empty.type_line("/plot");
    assert!(empty.screen().contains("no session is open"));
}

fn action(tool: &str, port: &str, detail: &str) -> serialbot_mcp::guard::Action {
    serialbot_mcp::guard::Action {
        tool: tool.into(),
        session: None,
        port: port.into(),
        detail: detail.into(),
    }
}

#[test]
fn the_ai_panel_asks_and_lists_what_the_agent_did() {
    use crate::commands::mcp::{Event, Events, send};
    use serialbot_mcp::approval::Approvals;
    use serialbot_mcp::guard::Guard;
    use std::sync::Mutex;

    let mut r = Rig::new(&["mock0"], 140, 30);
    let (tx, rx) = std::sync::mpsc::channel();
    let events: Events = Arc::new(Mutex::new(tx));
    let notify = Arc::clone(&events);
    let approvals = Arc::new(Approvals::new(
        Duration::from_secs(30),
        Box::new(move |queue| send(&notify, Event::Queue(queue))),
    ));
    r.tui.ai = Some(Ai::from_parts(
        "http://127.0.0.1:7777/mcp".into(),
        "secret-token".into(),
        Some(Arc::clone(&approvals)),
        rx,
    ));
    r.tui.tick();
    r.draw();
    assert!(
        r.screen().contains("http://127.0.0.1:7777/mcp"),
        "{}",
        r.screen()
    );
    assert!(r.screen().contains("token: secret-token"));
    assert!(r.screen().contains("no agent action yet"));

    // An agent wants to write: the panel and the input row ask.
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let asked = Arc::clone(&approvals);
    let first = runtime.spawn(async move {
        asked
            .approve(action("io.write", "mock0", "\"AT\\r\\n\""))
            .await
    });
    r.until_shows("AI wants io.write mock0 \"AT\\r\\n\"  y allow");
    assert!(
        r.screen().contains("? AI wants io.write mock0"),
        "{}",
        r.screen()
    );
    // Typing does not reach the input box meanwhile.
    r.keys(&[press(KeyCode::Char('h'))]);
    assert!(r.tui.keys.editor.is_empty());
    r.keys(&[press(KeyCode::Char('y'))]);
    assert_eq!(runtime.block_on(first).unwrap(), Ok(()));
    send(
        &events,
        Event::Done(action("io.write", "mock0", "\"AT\\r\\n\""), Ok(())),
    );
    r.until_shows("✓ io.write mock0");

    let asked = Arc::clone(&approvals);
    let second =
        runtime.spawn(async move { asked.approve(action("signal.reset", "mock0", "")).await });
    r.until_shows("AI wants signal.reset mock0");
    r.keys(&[press(KeyCode::Esc)]);
    assert!(runtime.block_on(second).unwrap().is_err());
    send(
        &events,
        Event::Done(
            action("signal.reset", "mock0", ""),
            Err("refused by the user".into()),
        ),
    );
    r.until_shows("✗ signal.reset mock0 — refused");
    // Typing works again.
    r.keys(&[press(KeyCode::Char('h'))]);
    assert_eq!(r.tui.keys.editor.text(), "h");
    let mut ai = r.tui.ai.take().unwrap();
    assert!(!ai.answer(Decision::Once), "nothing is waiting");
    ai.stop();
}

#[test]
fn tui_with_mcp_shows_the_sessions_agents_open() {
    use rmcp::ServiceExt;
    use rmcp::model::{CallToolRequestParams, ClientConfig};
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;

    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let port_arg = port.to_string();
    let f = Fixture::new();
    let mut tui = start(
        &f,
        &[
            "--mcp",
            "--approve",
            "all",
            "--mcp-port",
            &port_arg,
            "mock0",
        ],
        false,
        Lang::En,
    );
    let token = tui.ai.as_ref().unwrap().token.clone();
    assert!(tui.ai.as_ref().unwrap().url.contains(&port_arg));
    let saved = crate::config::CliConfig::load(&f.config_dir).unwrap();
    assert_eq!(saved.mcp_token.as_deref(), Some(token.as_str()));

    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(format!("http://127.0.0.1:{port}/mcp"))
                .auth_header(token),
        );
        let client = ClientConfig::default().serve(transport).await.unwrap();
        let serde_json::Value::Object(args) =
            serde_json::json!({"port": "mock1", "baud_rate": 9600})
        else {
            unreachable!()
        };
        let opened = client
            .call_tool(CallToolRequestParams::new("session.open").with_arguments(args))
            .await
            .unwrap();
        assert_ne!(opened.is_error, Some(true), "{opened:?}");
        client.cancel().await.unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while tui.panes.len() < 2 || tui.ai.as_ref().unwrap().steps.is_empty() {
        assert!(Instant::now() < deadline, "{tui:?}");
        tui.tick();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(tui.panes[1].name, "mock1");
    assert_eq!(tui.shown, [0, 1]);
    assert!(tui.panes[1].unread);
    assert_eq!(tui.focus, 0, "the user's focus stays");
    assert!(
        tui.ai.as_ref().unwrap().steps[0]
            .text
            .starts_with("session.open mock1")
    );
    let status = tui.panes[1].console.status_parts();
    assert!(
        status[1].starts_with("RX "),
        "no speed is known for it: {status:?}"
    );
    tui.close_all();
    assert!(f.manager.list().is_empty());
}

const EXPORT: &str = r#"[
  {"name": "boot", "commands": [
    {"name": "hello", "text": "AT", "ending": "crlf", "delayMs": 0},
    {"text": "AT+GMR", "ending": "lf", "delayMs": 20}
  ]},
  {"name": "raw", "commands": [{"name": "ping", "text": "01 02", "hex": true}]}
]"#;

fn export_file(f: &Fixture) -> String {
    std::fs::create_dir_all(&f.config_dir).unwrap();
    let path = f.config_dir.join("commands.json");
    std::fs::write(&path, EXPORT).unwrap();
    path.display().to_string()
}

#[test]
fn command_groups_send_by_click_or_command() {
    let mut r = Rig::new(&["mock0"], 120, 30);
    r.type_line("/cmd 1");
    assert!(
        r.logged(0, "no command groups"),
        "{:?}",
        r.tui.panes[0].lines
    );
    r.type_line("/groups");
    r.type_line("/group x");
    assert!(r.logged(0, "/groups <file>"));
    r.type_line("/groups /nonexistent.json");
    assert!(r.logged(0, "/nonexistent.json:"));
    let file = export_file(&r.f);
    r.type_line(&format!("/groups {file}"));
    assert!(r.screen().contains("Commands"), "{}", r.screen());
    assert!(r.screen().contains("boot ‹1/2›"), "{}", r.screen());
    assert!(r.screen().contains("1 hello") && r.screen().contains("2 AT+GMR"));

    let device = r.device("mock0");
    r.type_line("/cmd 1");
    assert_eq!(device.take_written(), b"AT\r\n");
    r.type_line("/cmd 9");
    assert!(r.logged(0, "/cmd 9: 1-2 or all"));
    // A click on a command sends it to the focused session.
    let area = r.tui.regions.groups;
    r.click(area.x + 2, area.y + 3);
    assert_eq!(device.take_written(), b"AT+GMR\n");
    // A click on the header goes to the next group.
    r.click(area.x + 2, area.y + 1);
    assert!(r.screen().contains("raw ‹2/2›"), "{}", r.screen());
    r.type_line("/cmd all");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !r.logged(0, "script finished") {
        assert!(Instant::now() < deadline, "{:?}", r.tui.panes[0].lines);
        r.tui.tick();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(device.take_written(), [0x01, 0x02]);
    r.type_line("/group BOOT");
    assert_eq!(r.tui.groups.as_ref().unwrap().current, 0);
    r.type_line("/group 2");
    assert_eq!(r.tui.groups.as_ref().unwrap().current, 1);
    r.type_line("/group nope");
    assert!(r.logged(0, "/group nope: boot, raw"));
    r.type_line("/groups");
    assert_eq!(r.tui.regions.groups, Rect::default(), "hidden");
}

#[test]
fn groups_from_the_command_line() {
    let f = Fixture::new();
    let file = export_file(&f);
    let tui = start(&f, &["--groups", &file, "mock0"], false, Lang::En);
    assert_eq!(tui.groups.as_ref().unwrap().groups.len(), 2);
    let line = vec!["serialbot-cli", "tui", "--groups", "/nonexistent.json"];
    let Command::Tui(args) = Cli::try_parse_from(line).unwrap().command else {
        panic!("not tui");
    };
    let e = prepare(&args, &f.manager, &env(&f, false, Lang::En)).unwrap_err();
    assert_eq!(e.exit, crate::exit::Exit::Usage);
}

#[test]
fn dragging_a_border_resizes_the_panes() {
    let mut r = Rig::new(&["mock0", "mock1"], 120, 30);
    let (_, left) = r.tui.regions.panes[0];
    let border = left.right() - 1;
    r.mouse(MouseEventKind::Down(MouseButton::Left), border, 5);
    r.mouse(MouseEventKind::Drag(MouseButton::Left), border + 20, 5);
    r.mouse(MouseEventKind::Up(MouseButton::Left), border + 20, 5);
    let (_, wider) = r.tui.regions.panes[0];
    assert_eq!(wider.width, left.width + 20);
    assert_eq!(r.tui.focus, 0, "dragging is not a click");
    // Panes keep a minimum size.
    r.mouse(
        MouseEventKind::Down(MouseButton::Left),
        wider.right() - 1,
        5,
    );
    r.mouse(MouseEventKind::Drag(MouseButton::Left), 200, 5);
    let (_, right) = r.tui.regions.panes[1];
    assert!(right.width >= 8, "{right:?}");
    // Stacked panes resize by rows.
    r.prefix(KeyCode::Char('-'));
    let (_, top) = r.tui.regions.panes[0];
    r.mouse(
        MouseEventKind::Down(MouseButton::Left),
        top.x + 5,
        top.bottom() - 1,
    );
    r.mouse(
        MouseEventKind::Drag(MouseButton::Left),
        top.x + 5,
        top.bottom() + 3,
    );
    let (_, taller) = r.tui.regions.panes[0];
    assert_eq!(taller.height, top.height + 4);
    // A new pane brings back equal sizes.
    r.type_line("/open mock2");
    assert_eq!(r.tui.weights(), [1, 1, 1]);
}
