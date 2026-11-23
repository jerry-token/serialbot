//! The real `serialbot-cli` program: `mcp serve` and a client talking to
//! it through `mcp stdio`, the way Claude Desktop does. Linux only: the
//! config directory is moved with `XDG_CONFIG_HOME`.
#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const TOKEN: &str = "e2e-token-0123456789abcdef";

struct Killed(Child);

impl Drop for Killed {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn config_home() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("serialbot-mcp-e2e-{}-{nanos}", std::process::id()))
}

fn cli(home: &PathBuf) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_serialbot-cli"));
    cmd.env("XDG_CONFIG_HOME", home).env("LANG", "C");
    cmd
}

/// Reads stdout lines on a thread so the test can time out.
fn lines(child: &mut Child) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

fn wait_for(rx: &mpsc::Receiver<String>, needle: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(line) if line.contains(needle) => return line,
            Ok(_) => {}
            Err(e) => panic!("no line with {needle:?}: {e}"),
        }
    }
}

#[test]
fn stdio_clients_reach_mcp_serve() {
    let home = config_home();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
        .to_string();
    let mut server = Killed(
        cli(&home)
            .args([
                "mcp",
                "serve",
                "--approve",
                "all",
                "--port",
                &port,
                "--token",
                TOKEN,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let banner = lines(&mut server.0);
    wait_for(&banner, "/mcp");
    // --token is not saved: the stdio bridge needs it in cli.json.
    let dir = home.join("cc.aiany.serialbot");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("cli.json"),
        format!(r#"{{"mcp_token": "{TOKEN}", "mcp_port": {port}}}"#),
    )
    .unwrap();

    let mut bridge = Killed(
        cli(&home)
            .args(["mcp", "stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let replies = lines(&mut bridge.0);
    let mut stdin = bridge.0.stdin.take().unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{{"name":"e2e","version":"1"}}}}}}"#
    )
    .unwrap();
    wait_for(&replies, r#""id":1"#);
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"session.list","arguments":{{}}}}}}"#
    )
    .unwrap();
    let listed = wait_for(&replies, r#""id":2"#);
    assert!(listed.contains("sessions"), "{listed}");

    // The client closing stdin ends the bridge.
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(10);
    while bridge.0.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "mcp stdio kept running after stdin closed"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = std::fs::remove_dir_all(&home);
}
