// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};

use super::targets;
use crate::config::CliConfig;
use crate::testing::{Fixture, Ran};
use serialbot_mcp::clients::Client as McpClient;
use serialbot_mcp::{DEFAULT_PORT, MCP_PATH};

const TOKEN: &str = "cli-test-token-0123456789";

/// A port nothing listens on right now.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

type Client = RunningService<RoleClient, ClientConfig>;

/// Connects once the server is up.
async fn client(port: u16) -> Client {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(format!("http://127.0.0.1:{port}/mcp"))
                .auth_header(TOKEN),
        );
        match ClientConfig::default().serve(transport).await {
            Ok(client) => return client,
            Err(e) if Instant::now() > deadline => panic!("cannot connect: {e}"),
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

async fn call(client: &Client, tool: &'static str, args: Value) -> CallToolResult {
    let Value::Object(args) = args else {
        panic!("arguments must be an object");
    };
    client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(args))
        .await
        .unwrap()
}

/// Runs `mcp serve <extra>` with the fixture, lets `agent` talk to it, then
/// stops it with Ctrl+C.
fn serve_with<F>(fixture: &Fixture, extra: &[&str], agent: impl FnOnce(u16) -> F) -> Ran
where
    F: std::future::Future<Output = ()>,
{
    let port = free_port();
    let port_arg = port.to_string();
    let mut args = vec!["mcp", "serve", "--port", &port_arg, "--token", TOKEN];
    args.extend_from_slice(extra);
    std::thread::scope(|scope| {
        let server = scope.spawn(|| fixture.run(&args));
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(agent(port));
        fixture.interrupted.store(true, Ordering::Relaxed);
        server.join().unwrap()
    })
}

#[test]
fn serves_the_tools_and_logs_what_agents_do() {
    let fixture = Fixture::new();
    let ran = serve_with(&fixture, &["--approve", "all"], |port| async move {
        let client = client(port).await;
        let open = call(
            &client,
            "session.open",
            json!({"port": "mock0", "baud_rate": 115_200}),
        )
        .await;
        assert_ne!(open.is_error, Some(true), "{open:?}");
        let list = call(&client, "port.list", json!({})).await;
        assert!(format!("{list:?}").contains("mock1"));
        client.cancel().await.unwrap();
    });
    assert_eq!(ran.code, 0, "{ran:?}");
    assert!(ran.out.contains(&format!("token: {TOKEN}")), "{}", ran.out);
    assert!(ran.out.contains("/mcp"), "{}", ran.out);
    assert!(ran.out.contains("mcp stdio"), "{}", ran.out);
    assert!(ran.err.contains("✓ AI session.open mock0"), "{}", ran.err);
    assert!(
        fixture
            .manager
            .list()
            .iter()
            .all(|s| s.state != serialbot_core::SessionState::Open)
    );
}

#[test]
fn asks_in_the_terminal() {
    let mut fixture = Fixture::new();
    fixture.interactive = true;
    fixture.stdin = b"maybe\ny\nn\n".to_vec();
    let ran = serve_with(&fixture, &[], |port| async move {
        let client = client(port).await;
        let first = call(
            &client,
            "session.open",
            json!({"port": "mock0", "baud_rate": 115_200}),
        )
        .await;
        assert_ne!(first.is_error, Some(true), "{first:?}");
        let second = call(
            &client,
            "session.open",
            json!({"port": "mock1", "baud_rate": 9600}),
        )
        .await;
        assert_eq!(second.is_error, Some(true), "{second:?}");
        client.cancel().await.unwrap();
    });
    assert_eq!(ran.code, 0, "{ran:?}");
    assert!(
        ran.err.contains("AI wants session.open mock0"),
        "{}",
        ran.err
    );
    assert!(ran.err.contains("y allow"), "{}", ran.err);
    assert!(ran.err.contains("✓ AI session.open mock0"), "{}", ran.err);
    assert!(ran.err.contains("✗ AI session.open mock1"), "{}", ran.err);
}

#[test]
fn deny_refuses_every_change() {
    let fixture = Fixture::new();
    let ran = serve_with(&fixture, &["--approve", "deny"], |port| async move {
        let client = client(port).await;
        let open = call(
            &client,
            "session.open",
            json!({"port": "mock0", "baud_rate": 115_200}),
        )
        .await;
        assert_eq!(open.is_error, Some(true));
        assert!(format!("{open:?}").contains("--approve deny"));
        client.cancel().await.unwrap();
    });
    assert_eq!(ran.code, 0, "{ran:?}");
    assert!(fixture.manager.list().is_empty());
}

#[test]
fn ask_needs_a_terminal() {
    let fixture = Fixture::new();
    let ran = fixture.run(&["mcp", "serve"]);
    assert_eq!(ran.code, 2);
    assert!(ran.err.contains("--approve all"), "{}", ran.err);
}

#[test]
fn short_tokens_are_refused() {
    let fixture = Fixture::new();
    let ran = fixture.run(&["mcp", "serve", "--approve", "all", "--token", "short"]);
    assert_eq!(ran.code, 2);
    assert!(ran.err.contains("at least"), "{}", ran.err);
}

#[test]
fn the_token_is_made_once_and_kept() {
    let fixture = Fixture::new();
    let port = free_port();
    let port_arg = port.to_string();
    let first = std::thread::scope(|scope| {
        let server =
            scope.spawn(|| fixture.run(&["mcp", "serve", "--approve", "all", "--port", &port_arg]));
        let deadline = Instant::now() + Duration::from_secs(10);
        while CliConfig::load(&fixture.config_dir)
            .unwrap()
            .mcp_token
            .is_none()
        {
            assert!(Instant::now() < deadline, "no token saved");
            std::thread::sleep(Duration::from_millis(20));
        }
        fixture.interrupted.store(true, Ordering::Relaxed);
        server.join().unwrap()
    });
    assert_eq!(first.code, 0, "{first:?}");
    let saved = CliConfig::load(&fixture.config_dir).unwrap();
    let token = saved.mcp_token.unwrap();
    assert!(first.out.contains(&format!("token: {token}")));
    assert_eq!(saved.mcp_port, Some(port));
    let (urls, found) = targets(Some(&fixture.config_dir)).remove(0);
    assert_eq!(found, token);
    assert_eq!(urls[0], format!("http://127.0.0.1:{port}/mcp"));
    assert_eq!(
        urls.len(),
        usize::from(serialbot_mcp::server::PORT_ATTEMPTS)
    );

    fixture.interrupted.store(false, Ordering::Relaxed);
    let second = std::thread::scope(|scope| {
        let server =
            scope.spawn(|| fixture.run(&["mcp", "serve", "--approve", "all", "--port", &port_arg]));
        std::thread::sleep(Duration::from_millis(300));
        fixture.interrupted.store(true, Ordering::Relaxed);
        server.join().unwrap()
    });
    assert!(
        second.out.contains(&format!("token: {token}")),
        "{second:?}"
    );
}

#[test]
fn stdio_looks_at_the_cli_server_first_then_the_app() {
    let dir = std::env::temp_dir().join(format!("serialbot-cli-targets-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(targets(None).is_empty());
    assert!(targets(Some(&dir)).is_empty());
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("mcp.json"),
        r#"{"enabled": true, "port": 7700, "token": "app-token-0123456789"}"#,
    )
    .unwrap();
    CliConfig {
        mcp_token: Some("cli-token-0123456789".into()),
        mcp_port: Some(7800),
    }
    .save(&dir)
    .unwrap();
    let found = targets(Some(&dir));
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].1, "cli-token-0123456789");
    assert_eq!(found[0].0[0], "http://127.0.0.1:7800/mcp");
    assert_eq!(found[1].1, "app-token-0123456789");
    assert_eq!(found[1].0[1], "http://127.0.0.1:7701/mcp");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stdio_without_a_server_says_how_to_start_one() {
    let fixture = Fixture::new();
    let ran = fixture.run(&["mcp", "stdio"]);
    assert_eq!(ran.code, 3);
    assert!(ran.err.contains("mcp serve"), "{}", ran.err);

    CliConfig {
        mcp_token: Some(TOKEN.into()),
        mcp_port: Some(free_port()),
    }
    .save(&fixture.config_dir)
    .unwrap();
    let ran = fixture.run(&["mcp", "stdio"]);
    assert_eq!(ran.code, 3, "{ran:?}");
    assert!(ran.err.contains("mcp serve"), "{}", ran.err);
}

#[test]
fn zh_texts() {
    let mut fixture = Fixture::new();
    fixture.lang = crate::Lang::Zh;
    let ran = fixture.run(&["mcp", "serve"]);
    assert!(ran.err.contains("需要终端"), "{}", ran.err);
    assert!(fixture.run(&["mcp", "--help"]).out.contains("MCP 服务"));
}

#[test]
fn install_writes_each_client_with_the_cli_token() {
    let fixture = Fixture::new();
    let dirs = fixture.client_dirs();

    let cursor = fixture.run(&["mcp", "install", "cursor"]);
    assert_eq!(cursor.code, 0, "{cursor:?}");
    let token = CliConfig::load(&fixture.config_dir)
        .unwrap()
        .mcp_token
        .unwrap();
    let path = McpClient::Cursor.config_path(&dirs);
    assert!(
        cursor
            .out
            .contains(&format!("Added SerialBot to Cursor: {}", path.display()))
    );
    assert!(cursor.err.contains("mcp serve"), "{cursor:?}");
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        value["mcpServers"]["serialbot"]["headers"]["Authorization"],
        format!("Bearer {token}")
    );
    assert_eq!(
        value["mcpServers"]["serialbot"]["url"],
        format!("http://127.0.0.1:{DEFAULT_PORT}{MCP_PATH}")
    );

    // Again: nothing to change.
    let again = fixture.run(&["mcp", "install", "cursor"]);
    assert!(again.out.contains("already set up"), "{again:?}");

    // Claude Desktop starts this program's stdio bridge; the old file is backed up.
    let claude = McpClient::ClaudeDesktop.config_path(&dirs);
    std::fs::create_dir_all(claude.parent().unwrap()).unwrap();
    std::fs::write(
        &claude,
        "{\"mcpServers\": {\"other\": {\"command\": \"x\"}}}",
    )
    .unwrap();
    let desktop = fixture.run(&["mcp", "install", "claude-desktop"]);
    assert_eq!(desktop.code, 0, "{desktop:?}");
    assert!(
        desktop.err.contains("previous file saved as"),
        "{desktop:?}"
    );
    assert!(!desktop.err.contains("mcp serve"), "{desktop:?}");
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&claude).unwrap()).unwrap();
    assert_eq!(value["mcpServers"]["other"]["command"], "x");
    assert_eq!(
        value["mcpServers"]["serialbot"]["args"],
        json!(["mcp", "stdio"])
    );
}

#[test]
fn install_prints_the_entry_when_it_cannot_write() {
    let fixture = Fixture::new();
    let path = McpClient::VsCode.config_path(&fixture.client_dirs());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{ // comments\n}").unwrap();
    let ran = fixture.run(&["mcp", "install", "vscode"]);
    assert_eq!(ran.code, 2, "{ran:?}");
    assert!(ran.err.contains("by hand"), "{ran:?}");
    assert!(ran.out.contains("\"servers\""), "{ran:?}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ // comments\n}");

    let printed = fixture.run(&["mcp", "install", "vscode", "--print"]);
    assert_eq!(printed.code, 0, "{printed:?}");
    assert!(printed.out.contains("\"type\": \"http\""), "{printed:?}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ // comments\n}");
}

#[test]
fn install_knows_codex_gemini_and_opencode() {
    let fixture = Fixture::new();
    let dirs = fixture.client_dirs();
    for (arg, client) in [
        ("codex", McpClient::Codex),
        ("gemini", McpClient::Gemini),
        ("opencode", McpClient::OpenCode),
    ] {
        let ran = fixture.run(&["mcp", "install", arg]);
        assert_eq!(ran.code, 0, "{ran:?}");
        assert!(ran.out.contains(client.name()), "{ran:?}");
        let text = std::fs::read_to_string(client.config_path(&dirs)).unwrap();
        assert!(text.contains("serialbot"), "{arg}: {text}");
    }
}
