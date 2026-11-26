// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serialbot_core::transport::mock::MockDevice;
use serialbot_core::transport::serial::SerialConfig;

use super::*;
use crate::args::Ending;
use crate::i18n::Lang;
use crate::term::console::{default_capture_name, human_bytes, short_port};
use crate::testing::Fixture;

/// An interactive session on `mock0`, a 60×12 terminal emulator, and the
/// device.
struct Rig {
    f: Fixture,
    app: Interactive,
    vt: vt100::Parser,
    device: MockDevice,
}

fn settings() -> Settings {
    Settings {
        port: "mock0".into(),
        baud: 115_200,
        frame: "8N1".into(),
        encoding: "utf-8".into(),
        ending: Ending::Crlf,
        reset: crate::commands::signal::parse_steps("dtr=0 rts=1 10ms rts=0").unwrap(),
        reconnect: true,
        raw: false,
        escape: 'a',
        lang: Lang::En,
        color: false,
        utc_offset_min: 0,
    }
}

impl Rig {
    fn new() -> Self {
        Self::with(settings())
    }

    fn with(settings: Settings) -> Self {
        let f = Fixture::new();
        let session = f
            .manager
            .open(&SerialConfig::new("mock0", 115_200), None)
            .unwrap();
        let device = f.devices.lock().unwrap()["mock0"].clone();
        let mut app = Interactive::new(session, Arc::clone(&f.manager), settings, 100).unwrap();
        let mut vt = vt100::Parser::new(40, 100, 200);
        let mut out = Vec::new();
        app.start(&mut out).unwrap();
        vt.process(&out);
        Self { f, app, vt, device }
    }

    fn keys(&mut self, keys: &[KeyEvent]) -> Flow {
        let mut flow = Flow::Continue;
        for key in keys {
            let mut out = Vec::new();
            flow = self.app.key(&mut out, *key).unwrap();
            self.vt.process(&out);
        }
        flow
    }

    fn type_line(&mut self, text: &str) -> Flow {
        let mut keys: Vec<KeyEvent> = text.chars().map(|c| press(KeyCode::Char(c))).collect();
        keys.push(press(KeyCode::Enter));
        self.keys(&keys)
    }

    fn prefix(&mut self, c: char) -> Flow {
        self.keys(&[ctrl('a'), press(KeyCode::Char(c))])
    }

    /// Ticks until `done` or five seconds.
    fn until(&mut self, mut done: impl FnMut(&Self) -> bool) -> Flow {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut out = Vec::new();
            let flow = self.app.tick(&mut out).unwrap();
            self.vt.process(&out);
            if done(self) || flow != Flow::Continue {
                return flow;
            }
            assert!(
                Instant::now() < deadline,
                "timed out; screen:\n{}",
                self.screen()
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn screen(&self) -> String {
        self.vt.screen().contents()
    }

    fn shows(&self, text: &str) -> bool {
        self.screen().contains(text)
    }
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

#[test]
fn typed_lines_are_sent_and_echoed_above_the_input_box() {
    let mut r = Rig::new();
    assert!(r.shows("mock0  115200 8N1  RX 0 TX 0"), "{}", r.screen());
    r.type_line("AT");
    assert_eq!(r.device.take_written(), b"AT\r\n");
    r.until(|r| r.shows("→ AT\\r\\n"));
    assert!(r.shows("TX 4"), "{}", r.screen());
    let rows: Vec<String> =
        r.vt.screen()
            .rows(0, 100)
            .map(|l| l.trim_end().to_owned())
            .collect();
    let echo = rows.iter().position(|l| l == "→ AT\\r\\n").unwrap();
    assert_eq!(rows[echo + 1], "›", "the input box follows the output");
    assert!(format!("{:?}", r.app).contains("Interactive"));
}

#[test]
fn received_lines_scroll_and_the_unfinished_one_waits_above_the_box() {
    let mut r = Rig::new();
    r.device.send(b"boot ok\r\nuart:~$ ");
    r.until(|r| r.shows("uart:~$"));
    let rows: Vec<String> =
        r.vt.screen()
            .rows(0, 100)
            .map(|l| l.trim_end().to_owned())
            .collect();
    assert_eq!(rows[..3], ["boot ok", "uart:~$", "›"], "{rows:?}");
    // Typing does not disturb the pending line.
    r.keys(&[press(KeyCode::Char('l'))]);
    assert!(r.shows("uart:~$ \n› l"), "{}", r.screen());
}

#[test]
fn help_quit_and_unknown_keys() {
    let mut r = Rig::new();
    r.prefix('z');
    assert!(r.shows("Ctrl+A then: Z help"), "{}", r.screen());
    r.prefix('`');
    assert!(r.shows("Ctrl+A ` has no command"), "{}", r.screen());
    r.prefix('v');
    assert!(r.shows("works in serialbot-cli tui"), "{}", r.screen());
    r.prefix('d');
    assert!(r.shows("Ctrl+A D is a modem key"), "{}", r.screen());
    r.keys(&[ctrl('a')]);
    assert!(r.shows("Ctrl+A …"), "the status shows the pending prefix");
    assert_eq!(r.keys(&[press(KeyCode::Char('x'))]), Flow::Quit);
    assert_eq!(r.prefix('q'), Flow::Quit);
    assert_eq!(r.type_line("/quit"), Flow::Quit);
    assert_eq!(r.prefix('j'), Flow::Shell);
    r.type_line("/nope");
    assert!(r.shows("/nope: unknown command"), "{}", r.screen());
}

#[test]
fn lines_and_baud_rate() {
    let mut r = Rig::new();
    r.type_line("/baud 9600");
    assert_eq!(r.device.baud_rate(), 9600);
    assert!(
        r.shows("baud rate 9600") && r.shows("mock0  9600 8N1"),
        "{}",
        r.screen()
    );
    r.type_line("/baud 0");
    assert!(r.shows("unknown command"), "{}", r.screen());
    r.prefix('p');
    assert_eq!(r.app.keys.editor.text(), "/baud ");
    r.keys(&[press(KeyCode::Esc)]);
    r.prefix('s');
    assert_eq!(r.app.keys.editor.text(), "/send-file ");
}

#[test]
fn modem_lines_breaks_and_the_reset_sequence() {
    let mut r = Rig::new();
    r.prefix('1');
    assert!(
        !r.device.dtr(),
        "DTR assumed on after opening, so it goes off"
    );
    r.type_line("/dtr");
    assert!(r.device.dtr());
    r.prefix('2');
    assert!(!r.device.rts());
    r.type_line("/rts");
    assert!(r.device.rts());
    r.prefix('3');
    assert!(!r.device.dtr() && !r.device.rts(), "dtr=0 rts=1 10ms rts=0");
    r.type_line("/reset");
    r.prefix('f');
    r.type_line("/break");
    assert_eq!(r.device.breaks().len(), 2);
    r.prefix('h');
    assert!(r.device.dtr(), "hang up ends with DTR on");
    assert!(r.shows("DTR● RTS○"), "{}", r.screen());
}

#[test]
fn saving_to_a_file() {
    let dir = std::env::temp_dir().join(format!("serialbot-cli-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("cap.log");
    let mut r = Rig::new();
    r.type_line(&format!("/log {}", path.display()));
    assert!(r.shows("saving to"), "{}", r.screen());
    assert!(r.shows("⏺"), "{}", r.screen());
    r.device.send(b"hello\n");
    r.until(|r| r.shows("hello"));
    r.prefix('l');
    assert!(r.shows("saved"), "{}", r.screen());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("RX hello\\n"), "{text}");
    r.type_line("/log /no/such/dir/x.log");
    assert!(r.shows("failed"), "{}", r.screen());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sending_a_file() {
    let dir = std::env::temp_dir().join(format!("serialbot-cli-sendfile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("fw.bin");
    std::fs::write(&path, vec![7u8; 3000]).unwrap();
    let mut r = Rig::new();
    r.app.console.feed.display.echo = false;
    r.type_line(&format!("/send-file {}", path.display()));
    r.until(|r| r.shows("sent fw.bin (3000 B)"));
    assert_eq!(r.device.take_written().len(), 3000);
    r.type_line("/send-file /no/such/file");
    assert!(r.shows("failed"), "{}", r.screen());

    // XMODEM to a bootloader that starts with C and ACKs everything.
    r.type_line(&format!("/send-file --xmodem {}", path.display()));
    assert!(
        r.shows("XMODEM: waiting for the device to start receiving"),
        "{}",
        r.screen()
    );
    assert!(r.shows("↑ XMODEM 0%"), "{}", r.screen());
    r.device.send(b"C");
    let mut blocks = 0;
    r.until(|r| {
        let written = r.device.take_written();
        if !written.is_empty() {
            blocks += 1;
            r.device.send(&[0x06]);
        }
        r.shows("sent fw.bin (3000 B)") && blocks > 24
    });
    r.type_line("/send-file --zmodem x");
    assert!(r.shows("--zmodem: unknown command"), "{}", r.screen());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn starting_in_raw_mode_says_how_to_leave_it() {
    let r = Rig::with(Settings {
        raw: true,
        ..settings()
    });
    assert!(
        r.shows("raw mode: every key goes to the device"),
        "{}",
        r.screen()
    );
    assert!(!r.shows("›"));
}

#[test]
fn raw_mode_passes_everything_through() {
    let mut r = Rig::new();
    r.type_line("/raw");
    assert!(r.shows("raw mode"), "{}", r.screen());
    r.keys(&[
        press(KeyCode::Char('l')),
        press(KeyCode::Char('s')),
        press(KeyCode::Enter),
    ]);
    assert_eq!(r.device.take_written(), b"ls\r");
    r.device.send(b"\x1b[1mfile.txt\x1b[0m\r\n");
    r.until(|r| r.shows("file.txt"));
    assert!(!r.shows("›"), "no input box in raw mode");
    let mut out = Vec::new();
    r.app.paste(&mut out, "cat x\r").unwrap();
    assert_eq!(r.device.take_written(), b"cat x\r");
    r.prefix('z');
    assert!(r.shows("Ctrl+A then"), "notes still show in raw mode");
    r.prefix('/');
    assert!(r.shows("back to the input box"), "{}", r.screen());
    assert_eq!(r.app.keys.editor.text(), "/");
}

#[test]
fn display_options() {
    let mut r = Rig::new();
    r.type_line("/hex");
    r.device.send(&[0x01, 0xff]);
    r.until(|r| r.shows("01 ff"));
    assert!(r.shows("HEX"));
    r.type_line("/hex");
    r.type_line("/time");
    r.device.send(b"t\n");
    r.until(|r| {
        r.vt.screen()
            .contents()
            .lines()
            .any(|l| l.starts_with('[') && l.ends_with("] t"))
    });
    r.prefix('n');
    r.prefix('n');
    r.prefix('e');
    r.type_line("quiet");
    assert!(!r.shows("→ quiet"), "echo is off");
    r.type_line("/echo");
    r.prefix('a');
    r.prefix('u');
    r.prefix('w');
    r.prefix('t');
    assert!(r.shows("display: hex off"), "{}", r.screen());
    assert!(r.shows("add LF on · wrap off"), "{}", r.screen());
    r.prefix('b');
    assert!(r.shows("Shift+PgUp"), "{}", r.screen());
    r.prefix('o');
    assert!(r.shows("later version"), "{}", r.screen());
    r.prefix('g');
    assert_eq!(r.app.keys.editor.text(), "/run ");
    r.keys(&[press(KeyCode::Esc)]);
    r.prefix('c');
    assert!(!r.shows("later version"), "cleared");
    r.type_line("/clear");
    r.type_line("/help");
    assert!(r.shows("Commands:"), "{}", r.screen());
}

#[test]
fn a_lost_device_is_waited_for_and_comes_back() {
    let mut r = Rig::new();
    r.device.disconnect();
    r.until(|r| r.shows("device disconnected"));
    assert!(r.shows("waiting for the device"), "{}", r.screen());
    r.until(|r| r.shows("reconnected"));
    let device = r.f.devices.lock().unwrap()["mock0"].clone();
    device.send(b"again\n");
    r.until(|r| r.shows("again"));
}

#[test]
fn without_reconnecting_a_lost_device_ends_the_session() {
    let mut r = Rig::with(Settings {
        reconnect: false,
        ..settings()
    });
    r.device.disconnect();
    assert_eq!(r.until(|_| false), Flow::Lost);
}

#[test]
fn search_resize_and_leaving() {
    let mut r = Rig::new();
    r.type_line("AT+GMR");
    r.keys(&[ctrl('r'), press(KeyCode::Char('G'))]);
    assert!(r.shows("(search) G → AT+GMR"), "{}", r.screen());
    r.keys(&[press(KeyCode::Esc)]);
    r.keys(&[
        press(KeyCode::Char('a')),
        KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
        press(KeyCode::Char('b')),
    ]);
    assert!(r.shows("› a\n  b"), "{}", r.screen());
    let mut out = Vec::new();
    r.app.resize(&mut out, 40).unwrap();
    r.device.send(b"partial");
    r.until(|r| r.shows("partial"));
    r.app.leave(&mut out).unwrap();
    r.vt.process(&out);
    assert!(!r.shows("›"), "{}", r.screen());
    r.app.stop_jobs();
    assert_eq!(human_bytes(999), "999");
    assert_eq!(short_port("/dev/ttyUSB0"), "/dev/ttyUSB0");
    assert_eq!(
        short_port("/dev/serial/by-id/usb-FTDI_FT232R-if00-port0"),
        "…-FTDI_FT232R-if00-port0"
    );
    assert_eq!(human_bytes(12_400), "12.4k");
    assert_eq!(human_bytes(3_100_000), "3.1M");
    assert!(
        default_capture_name()
            .to_string_lossy()
            .starts_with("serialbot-")
    );
}

#[test]
fn unknown_encodings_are_refused_and_colors_dim_the_status() {
    let f = Fixture::new();
    let session = f
        .manager
        .open(&SerialConfig::new("mock0", 115_200), None)
        .unwrap();
    let bad = Settings {
        encoding: "nope".into(),
        ..settings()
    };
    assert!(Interactive::new(Arc::clone(&session), Arc::clone(&f.manager), bad, 60).is_none());
    let colored = Settings {
        color: true,
        ..settings()
    };
    let app = Interactive::new(session, Arc::clone(&f.manager), colored, 60).unwrap();
    assert!(app.status().starts_with("\x1b[2m"));
}

#[test]
fn scripts_run_in_the_background_with_progress() {
    let dir = std::env::temp_dir().join(format!("serialbot-cli-script-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("check.txt");
    std::fs::write(&script, "send AT\nexpect OK\nmark checked\n").unwrap();
    let mut r = Rig::new();
    r.type_line(&format!("/run {}", script.display()));
    assert!(r.shows("running"), "{}", r.screen());
    assert!(r.shows("▶"), "the status shows a running script");
    r.type_line(&format!("/run {}", script.display()));
    assert!(r.shows("already running"), "{}", r.screen());
    let deadline = Instant::now() + Duration::from_secs(5);
    while r.device.take_written() != b"AT\r\n" {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    r.device.send(b"OK\r\n");
    r.until(|r| r.shows("script finished"));
    assert!(
        r.shows("✓ 1  send AT") && r.shows("✓ 2  expect OK"),
        "{}",
        r.screen()
    );
    assert!(r.shows("# checked"), "{}", r.screen());

    // A script that waits forever is stopped with /stop.
    std::fs::write(&script, "timeout 30s\nexpect never\n").unwrap();
    r.type_line(&format!("/run {}", script.display()));
    r.type_line("/stop");
    r.until(|r| r.shows("script stopped"));
    std::fs::write(&script, "timeout 10ms\nexpect never\n").unwrap();
    r.type_line(&format!("/run {}", script.display()));
    r.until(|r| r.shows("no reply matched"));
    std::fs::write(&script, "frobnicate\n").unwrap();
    r.type_line(&format!("/run {}", script.display()));
    assert!(r.shows("line 1"), "{}", r.screen());
    r.type_line("/run /no/such/script");
    assert!(r.shows("failed: /no/such/script"), "{}", r.screen());
    let json = dir.join("groups.json");
    std::fs::write(
        &json,
        r#"[{"name": "G", "commands": [{"text": "x", "ending": "none"}]}]"#,
    )
    .unwrap();
    r.type_line(&format!("/run {}", json.display()));
    r.until(|r| r.shows("script finished"));
    // Quitting stops a running script too.
    std::fs::write(&script, "wait 30s\n").unwrap();
    r.type_line(&format!("/run {}", script.display()));
    r.app.stop_jobs();
    // "script stopped" twice before, plus this one.
    r.until(|r| r.screen().matches("script stopped").count() == 3);
    let _ = std::fs::remove_dir_all(&dir);
}
