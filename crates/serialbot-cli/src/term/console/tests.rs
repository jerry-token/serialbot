use std::sync::Arc;
use std::time::{Duration, Instant};

use serialbot_core::transport::mock::MockDevice;
use serialbot_core::transport::serial::SerialConfig;

use super::*;
use crate::testing::Fixture;

fn console() -> (Fixture, Console, MockDevice) {
    let f = Fixture::new();
    let session = f
        .manager
        .open(&SerialConfig::new("mock0", 115_200), None)
        .unwrap();
    let device = f.devices.lock().unwrap()["mock0"].clone();
    let settings = Settings {
        port: "mock0".into(),
        baud: 115_200,
        frame: "8N1".into(),
        encoding: "utf-8".into(),
        ending: Ending::Lf,
        reset: Vec::new(),
        reconnect: true,
        raw: false,
        escape: 'a',
        lang: Lang::En,
        color: false,
        utc_offset_min: 0,
    };
    let console = Console::new(session, Arc::clone(&f.manager), settings).unwrap();
    (f, console, device)
}

/// Ticks until something arrives or five seconds pass.
fn tick_until(console: &mut Console, raw: bool) -> Tick {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let (tick, _) = console.tick(raw);
        if tick.rx > 0 || Instant::now() > deadline {
            return tick;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn ticks_give_lines_or_raw_bytes_and_count_what_arrived() {
    let (_f, mut c, device) = console();
    device.send(b"hello\r\n");
    let tick = tick_until(&mut c, false);
    assert_eq!(tick.lines, vec!["hello".to_owned()]);
    assert_eq!(tick.rx, 7);
    assert_eq!(tick.received, b"hello\r\n");
    assert!(tick.raw.is_empty());
    device.send(b"\x1b[2Jraw");
    let tick = tick_until(&mut c, true);
    assert_eq!(tick.raw, b"\x1b[2Jraw");
    assert!(tick.lines.is_empty());
    assert!(c.status_parts().iter().any(|p| p == "RX 14 TX 0"));
}

#[test]
fn commands_that_need_the_screen_come_back_as_effects() {
    let (_f, mut c, device) = console();
    let mut notes = Vec::new();
    assert_eq!(c.command(Slash::Clear, &mut notes), Some(Effect::Clear));
    assert_eq!(c.command(Slash::Raw, &mut notes), Some(Effect::Raw));
    assert_eq!(c.command(Slash::Quit, &mut notes), Some(Effect::Quit));
    assert_eq!(c.local(Local::Palette, &mut notes), Some(Effect::Palette));
    assert_eq!(c.local(Local::Shell, &mut notes), Some(Effect::Shell));
    assert_eq!(
        c.local(Local::RunScript, &mut notes),
        Some(Effect::Prefill("/run "))
    );
    assert_eq!(c.command(Slash::Hex, &mut notes), None);
    assert!(c.feed.display.hex);
    assert!(notes.is_empty());
    c.send_line("ping", &mut notes);
    assert_eq!(device.take_written(), b"ping\n");
    assert_eq!(c.notes(&["hi".into()]), vec!["· hi".to_owned()]);
    assert!(!c.is_lost());
    assert_eq!(c.settings().port, "mock0");
    assert_eq!(c.session().state(), SessionState::Open);
}
