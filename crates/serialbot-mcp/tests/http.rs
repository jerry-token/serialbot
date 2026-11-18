//! End-to-end tests: a real MCP client talks to the server over HTTP, and
//! the sessions run on mock devices.
#![allow(
    clippy::unwrap_used,
    reason = "test harness helpers outside #[test] functions"
)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};
use serialbot_core::transport::mock::{self, MockDevice};
use serialbot_core::transport::serial::{PortInfo, PortKind};
use serialbot_core::transport::{Connection, TransportError};
use serialbot_core::{SessionManager, SessionOptions, SystemClock};
use serialbot_mcp::{McpOptions, SerialTools, router, serve};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

const TOKEN: &str = "test-token-0123456789";

type Devices = Arc<Mutex<HashMap<String, MockDevice>>>;

struct Harness {
    addr: SocketAddr,
    devices: Devices,
    manager: Arc<SessionManager>,
    shutdown: CancellationToken,
}

impl Harness {
    /// Starts a server whose ports named `mock*` open as mock devices.
    async fn start() -> Self {
        let devices: Devices = Arc::default();
        let registry = Arc::clone(&devices);
        let opener = move |config: &serialbot_core::transport::serial::SerialConfig| -> Result<Connection, TransportError> {
            if !config.path.starts_with("mock") {
                return Err(TransportError::NotFound(config.path.clone()));
            }
            let (connection, device) = mock::connection(&config.path, Duration::from_millis(5));
            registry.lock().unwrap().insert(config.path.clone(), device);
            Ok(connection)
        };
        let manager = Arc::new(SessionManager::new(
            Box::new(opener),
            Arc::new(SystemClock),
            SessionOptions::default(),
        ));
        let lister = || {
            Ok(vec![
                PortInfo {
                    path: "mock0".into(),
                    kind: PortKind::Usb {
                        vid: 0x1a86,
                        pid: 0x7523,
                        serial_number: None,
                        manufacturer: Some("wch.cn".into()),
                        product: Some("USB Serial".into()),
                    },
                },
                PortInfo {
                    path: "mock1".into(),
                    kind: PortKind::Unknown,
                },
            ])
        };
        let tools = SerialTools::new(Arc::clone(&manager), Arc::new(lister));
        let shutdown = CancellationToken::new();
        let options = McpOptions::new(TOKEN).unwrap();
        let app = router(tools, &options, &shutdown);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(serve(listener, app, shutdown.clone()));
        Self {
            addr,
            devices,
            manager,
            shutdown,
        }
    }

    fn url(&self) -> String {
        format!("http://{}/mcp", self.addr)
    }

    async fn client(&self) -> RunningService<RoleClient, ClientConfig> {
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(self.url()).auth_header(TOKEN),
        );
        ClientConfig::default().serve(transport).await.unwrap()
    }

    fn device(&self, port: &str) -> MockDevice {
        self.devices.lock().unwrap()[port].clone()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.manager.close_all();
    }
}

async fn call(
    client: &RunningService<RoleClient, ClientConfig>,
    tool: &'static str,
    args: Value,
) -> CallToolResult {
    let Value::Object(args) = args else {
        panic!("arguments must be an object");
    };
    client
        .call_tool(CallToolRequestParams::new(tool).with_arguments(args))
        .await
        .unwrap()
}

/// Calls a tool that must succeed and returns its structured result.
async fn ok(
    client: &RunningService<RoleClient, ClientConfig>,
    tool: &'static str,
    args: Value,
) -> Value {
    let result = call(client, tool, args).await;
    assert_ne!(result.is_error, Some(true), "{tool} failed: {result:?}");
    result.structured_content.expect("structured result")
}

/// Calls a tool that must fail and returns its error text.
async fn err(
    client: &RunningService<RoleClient, ClientConfig>,
    tool: &'static str,
    args: Value,
) -> String {
    let result = call(client, tool, args).await;
    assert_eq!(result.is_error, Some(true), "{tool} succeeded: {result:?}");
    result.content[0]
        .as_text()
        .expect("text error")
        .text
        .clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_the_curated_tools_with_annotations() {
    let h = Harness::start().await;
    let client = h.client().await;
    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "io.read",
            "io.send_and_expect",
            "io.signals",
            "io.wait_for",
            "io.write",
            "port.list",
            "session.list",
            "session.open",
        ]
    );
    let hints: HashMap<&str, (Option<bool>, Option<bool>)> = tools
        .iter()
        .map(|t| {
            let a = t.annotations.as_ref().expect("annotations");
            (t.name.as_ref(), (a.read_only_hint, a.destructive_hint))
        })
        .collect();
    assert_eq!(hints["io.read"].0, Some(true));
    assert_eq!(hints["io.wait_for"].0, Some(true));
    assert_eq!(hints["io.write"], (Some(false), Some(true)));
    assert_eq!(hints["session.open"], (Some(false), Some(false)));
    for tool in &tools {
        assert!(
            tool.output_schema.is_some(),
            "{} has an output schema",
            tool.name
        );
    }
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn open_write_read_round_trip() {
    let h = Harness::start().await;
    let client = h.client().await;

    let ports = ok(&client, "port.list", json!({})).await;
    assert_eq!(ports["ports"][0]["port"], "mock0");
    assert_eq!(ports["ports"][0]["vid"], "1a86");
    assert_eq!(ports["ports"][0]["session"], Value::Null);
    assert_eq!(ports["ports"][1]["kind"], "unknown");

    let opened = ok(
        &client,
        "session.open",
        json!({"port": "mock0", "baud_rate": 921_600, "alias": "esp32"}),
    )
    .await;
    assert_eq!(opened["id"], "s1");
    assert_eq!(opened["alias"], "esp32");
    assert_eq!(opened["state"], "open");
    assert_eq!(opened["transport"], "mock");

    let ports = ok(&client, "port.list", json!({})).await;
    assert_eq!(ports["ports"][0]["session"], "s1");

    let written = ok(
        &client,
        "io.write",
        json!({"session": "esp32", "text": "AT\r\n"}),
    )
    .await;
    assert_eq!(written["bytes"], 4);
    assert_eq!(written["cursor"], 0);
    assert_eq!(written["seq"], 0);
    let device = h.device("mock0");
    assert_eq!(device.take_written(), b"AT\r\n");

    ok(
        &client,
        "io.write",
        json!({"session": "s1", "hex": "01 ff"}),
    )
    .await;
    assert_eq!(device.take_written(), [0x01, 0xff]);

    device.send(b"OK\r\n");
    let waited = ok(
        &client,
        "io.wait_for",
        json!({"session": "s1", "pattern": "OK", "cursor": 0, "timeout_ms": 2000}),
    )
    .await;
    assert_eq!(waited["matched"], true);
    assert_eq!(waited["data"], "OK");

    let read = ok(&client, "io.read", json!({"session": "s1"})).await;
    let entries = read["entries"].as_array().unwrap();
    let kinds: Vec<&str> = entries
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["tx", "tx", "rx"]);
    assert_eq!(entries[0]["data"], "AT\r\n");
    assert_eq!(entries[2]["data"], "OK\r\n");
    assert!(
        entries[2]["timestamp_us"].as_u64().unwrap()
            >= entries[0]["timestamp_us"].as_u64().unwrap()
    );
    assert_eq!(read["cursor"], 3);
    assert_eq!(read["missed"], 0);
    assert_eq!(read["more"], false);

    let hex = ok(
        &client,
        "io.read",
        json!({"session": "s1", "cursor": 1, "format": "hex"}),
    )
    .await;
    assert_eq!(hex["entries"][0]["data"], "01 ff");

    let limited = ok(&client, "io.read", json!({"session": "s1", "max_bytes": 1})).await;
    assert_eq!(
        limited["entries"].as_array().unwrap().len(),
        1,
        "at least one entry"
    );
    assert_eq!(limited["cursor"], 1);
    assert_eq!(limited["more"], true);

    let empty = ok(&client, "io.read", json!({"session": "s1", "cursor": 3})).await;
    assert_eq!(empty["entries"], json!([]));

    let sessions = ok(&client, "session.list", json!({})).await;
    assert_eq!(sessions["sessions"][0]["id"], "s1");
    assert_eq!(sessions["sessions"][0]["port"], "mock0");
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn signals_are_set_read_and_logged() {
    let h = Harness::start().await;
    let client = h.client().await;
    ok(&client, "session.open", json!({"port": "mock0"})).await;
    let device = h.device("mock0");
    device.set_line_states(serialbot_core::transport::LineStates {
        cts: true,
        dsr: false,
        ri: false,
        cd: true,
    });
    let read = ok(&client, "io.signals", json!({"session": "s1"})).await;
    assert_eq!(read["dtr"], Value::Null, "never set");
    assert_eq!(read["rts_manual"], true);
    assert_eq!(read["cts"], true);
    assert_eq!(read["cd"], true);
    assert_eq!(read["input_error"], Value::Null);

    let set = ok(
        &client,
        "io.signals",
        json!({"session": "s1", "dtr": false, "rts": true}),
    )
    .await;
    assert_eq!(set["dtr"], false);
    assert_eq!(set["rts"], true);
    assert!(!device.dtr() && device.rts());
    let log = ok(&client, "io.read", json!({"session": "s1"})).await;
    let markers: Vec<&str> = log["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["data"].as_str().unwrap())
        .collect();
    assert_eq!(markers, ["DTR=0", "RTS=1"]);

    device.disconnect();
    for _ in 0..200 {
        if ok(&client, "session.list", json!({})).await["sessions"][0]["state"] == "disconnected" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        err(
            &client,
            "io.signals",
            json!({"session": "s1", "rts": false})
        )
        .await
        .contains("Disconnected")
    );
    let read = ok(&client, "io.signals", json!({"session": "s1"})).await;
    assert_eq!(read["cts"], Value::Null);
    assert!(
        read["input_error"]
            .as_str()
            .unwrap()
            .contains("Disconnected")
    );
    assert!(
        err(&client, "io.signals", json!({"session": "nope"}))
            .await
            .contains("no session")
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn send_and_expect_catches_the_reply() {
    let h = Harness::start().await;
    let client = h.client().await;
    ok(&client, "session.open", json!({"port": "mock0"})).await;
    let device = h.device("mock0");
    let responder = tokio::task::spawn_blocking(move || {
        for _ in 0..400 {
            if device.take_written() == b"VER?\n" {
                device.send(b"noise\r\nVERSION 1.2.3\r\n");
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("command never arrived");
    });
    let reply = ok(
        &client,
        "io.send_and_expect",
        json!({"session": "s1", "text": "VER?\n", "pattern": r"VERSION (\S+)", "timeout_ms": 3000}),
    )
    .await;
    responder.await.unwrap();
    assert_eq!(reply["matched"], true);
    assert_eq!(reply["data"], "VERSION 1.2.3");
    assert!(reply["seq"].as_u64().is_some());
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn waits_time_out_without_a_match() {
    let h = Harness::start().await;
    let client = h.client().await;
    ok(&client, "session.open", json!({"port": "mock0"})).await;
    let waited = ok(
        &client,
        "io.wait_for",
        json!({"session": "s1", "pattern": "never", "timeout_ms": 50}),
    )
    .await;
    assert_eq!(waited["matched"], false);
    assert_eq!(waited["data"], Value::Null);
    assert_eq!(waited["cursor"], 0);
    let expected = ok(
        &client,
        "io.send_and_expect",
        json!({"session": "s1", "hex": "00", "pattern": "never", "timeout_ms": 50, "format": "hex"}),
    )
    .await;
    assert_eq!(expected["matched"], false);
    assert_eq!(expected["cursor"], 0);
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn tool_errors_are_reported_to_the_agent() {
    let h = Harness::start().await;
    let client = h.client().await;
    assert!(
        err(&client, "io.read", json!({"session": "s9"}))
            .await
            .contains("no session")
    );
    assert!(
        err(&client, "session.open", json!({"port": "ttyX"}))
            .await
            .contains("ttyX")
    );
    assert!(
        err(
            &client,
            "session.open",
            json!({"port": "mock0", "data_bits": 9})
        )
        .await
        .contains("data bits")
    );
    assert!(
        err(
            &client,
            "session.open",
            json!({"port": "mock0", "stop_bits": 3})
        )
        .await
        .contains("stop bits")
    );
    assert!(
        err(
            &client,
            "session.open",
            json!({"port": "mock0", "alias": "s7"})
        )
        .await
        .contains("looks like a session id")
    );

    ok(
        &client,
        "session.open",
        json!({"port": "mock0", "data_bits": 7, "parity": "even", "stop_bits": 2, "flow_control": "hardware"}),
    )
    .await;
    assert!(
        err(&client, "session.open", json!({"port": "mock0"}))
            .await
            .contains("already open")
    );
    assert!(
        err(&client, "io.write", json!({"session": "s1"}))
            .await
            .contains("`text` or `hex`")
    );
    assert!(
        err(&client, "io.write", json!({"session": "s1", "hex": "zz"}))
            .await
            .contains("invalid hex")
    );
    assert!(
        err(&client, "io.read", json!({"session": "s1", "max_bytes": 0}))
            .await
            .contains("max_bytes")
    );
    assert!(
        err(&client, "io.read", json!({"session": "s1", "cursor": 99}))
            .await
            .contains("ahead")
    );
    assert!(
        err(
            &client,
            "io.wait_for",
            json!({"session": "s1", "pattern": "("})
        )
        .await
        .contains("invalid pattern")
    );
    assert!(
        err(
            &client,
            "io.wait_for",
            json!({"session": "s1", "pattern": "x", "timeout_ms": 60001})
        )
        .await
        .contains("timeout_ms")
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn tools_report_unknown_and_disconnected_sessions() {
    let h = Harness::start().await;
    let client = h.client().await;
    ok(&client, "session.open", json!({"port": "mock0"})).await;
    assert!(
        err(
            &client,
            "io.send_and_expect",
            json!({"session": "nope", "text": "x", "pattern": "x"})
        )
        .await
        .contains("no session")
    );

    h.device("mock0").disconnect();
    let mut state = Value::Null;
    for _ in 0..200 {
        state = ok(&client, "session.list", json!({})).await["sessions"][0]["state"].clone();
        if state == "disconnected" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(state, "disconnected");
    assert!(
        err(&client, "io.write", json!({"session": "s1", "text": "x"}))
            .await
            .contains("session is Disconnected")
    );
    assert!(
        err(
            &client,
            "io.wait_for",
            json!({"session": "s1", "pattern": "x"})
        )
        .await
        .contains("session is Disconnected")
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn port_listing_errors_are_reported() {
    let manager = Arc::new(SessionManager::new(
        Box::new(|_: &serialbot_core::transport::serial::SerialConfig| {
            Err(TransportError::NotFound("x".into()))
        }),
        Arc::new(SystemClock),
        SessionOptions::default(),
    ));
    let tools = SerialTools::new(
        manager,
        Arc::new(|| Err(TransportError::PermissionDenied("udev".into()))),
    );
    let shutdown = CancellationToken::new();
    let app = router(tools, &McpOptions::new(TOKEN).unwrap(), &shutdown);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(serve(listener, app, shutdown.clone()));
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp"))
            .auth_header(TOKEN),
    );
    let client = ClientConfig::default().serve(transport).await.unwrap();
    assert!(err(&client, "port.list", json!({})).await.contains("udev"));
    client.cancel().await.unwrap();
    shutdown.cancel();
}

/// Sends a raw HTTP/1.1 `initialize` POST and returns the status code.
async fn raw_status(addr: SocketAddr, extra_headers: &str) -> u16 {
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
    let request = format!(
        "POST /mcp HTTP/1.1\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\n{extra_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = vec![0u8; 64];
    let n = stream.read(&mut response).await.unwrap();
    let head = String::from_utf8_lossy(&response[..n]).into_owned();
    head.split(' ').nth(1).unwrap().parse().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn http_requests_are_authenticated_and_validated() {
    let h = Harness::start().await;
    let host = format!("Host: 127.0.0.1:{}\r\n", h.addr.port());
    let auth = format!("Authorization: Bearer {TOKEN}\r\n");

    assert_eq!(raw_status(h.addr, &host).await, 401, "no token");
    assert_eq!(
        raw_status(
            h.addr,
            &format!("{host}Authorization: Bearer wrong-token-000000\r\n")
        )
        .await,
        401,
        "wrong token"
    );
    assert_eq!(
        raw_status(h.addr, &format!("{host}{auth}")).await,
        200,
        "valid"
    );
    assert_eq!(
        raw_status(h.addr, &format!("Host: localhost\r\n{auth}")).await,
        200,
        "localhost"
    );
    assert_eq!(
        raw_status(h.addr, &format!("Host: evil.example:9633\r\n{auth}")).await,
        403,
        "DNS rebinding: foreign Host"
    );
    assert_eq!(
        raw_status(
            h.addr,
            &format!("{host}{auth}Origin: https://evil.example\r\n")
        )
        .await,
        403,
        "browser page: any Origin"
    );
}
