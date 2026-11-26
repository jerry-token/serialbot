// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! The MCP server embedded in the app.
//!
//! It serves the same [`SessionManager`] as the window, so a port opened in
//! the UI can be driven by an agent and everything the agent sends shows up
//! in the UI. It listens on 127.0.0.1, or on the local network when the
//! settings say so. Running the server is [`RunningServer`]'s job; this
//! module keeps the settings (`mcp.json`) and tells the windows.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use serialbot_core::SessionManager;
use serialbot_mcp::clients::{self, Client};
use serialbot_mcp::server::RunningServer;
pub use serialbot_mcp::server::{PORT_ATTEMPTS, lan_address};
use serialbot_mcp::{PortLister, SerialTools};
use tokio::sync::Mutex;

use crate::approval::Approvals;
use crate::settings::{self, McpSettings};

/// What adding SerialBot to an MCP client did (see
/// [`McpController::install_client`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInstall {
    /// The client's product name.
    pub client: String,
    /// Its configuration file.
    pub path: String,
    /// Where the previous content was saved, if the file changed.
    pub backup: Option<String>,
    /// False when the file already held the entry.
    pub changed: bool,
    /// Why nothing was written, if so.
    pub error: Option<String>,
    /// The entry to add by hand when nothing was written.
    pub snippet: Option<String>,
}

/// What the UI shows about the MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    /// The user's settings.
    pub settings: McpSettings,
    /// Whether the server is listening.
    pub running: bool,
    /// Endpoint URL on this computer while running.
    pub url: Option<String>,
    /// Endpoint URL for other computers while running on the local network
    /// (this computer's address on its main network), if it can be found.
    pub lan_url: Option<String>,
    /// Why the server is not running although enabled, or why the settings
    /// file was ignored.
    pub error: Option<String>,
    /// Program that stdio-only MCP clients start, with the argument `mcp`
    /// (see [`crate::cli`]).
    pub stdio_command: Option<String>,
}

struct Inner {
    settings: McpSettings,
    running: Option<RunningServer>,
    error: Option<String>,
}

/// Starts, stops and reconfigures the embedded server.
pub struct McpController {
    manager: Arc<SessionManager>,
    list_ports: Arc<PortLister>,
    /// Asked before every change an agent makes.
    approvals: Arc<Approvals>,
    /// Where settings are saved; `None` keeps them in memory (tests).
    settings_path: Option<PathBuf>,
    inner: Mutex<Inner>,
    /// Told the new status whenever the server starts, stops or changes.
    on_change: Option<StatusListener>,
}

/// Receives the MCP status after every change (the app forwards it to all
/// windows).
pub type StatusListener = Box<dyn Fn(McpStatus) + Send + Sync>;

impl std::fmt::Debug for McpController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpController")
            .field("settings_path", &self.settings_path)
            .finish_non_exhaustive()
    }
}

impl McpController {
    /// A stopped controller. `note` is shown as the error until the next
    /// change (for example why the settings file was ignored). Agents'
    /// changes go through `approvals` while the settings ask to confirm.
    pub fn new(
        manager: Arc<SessionManager>,
        list_ports: Arc<PortLister>,
        approvals: Arc<Approvals>,
        settings: McpSettings,
        settings_path: Option<PathBuf>,
        note: Option<String>,
    ) -> Self {
        approvals.set_enabled(settings.confirm);
        Self {
            manager,
            list_ports,
            approvals,
            settings_path,
            inner: Mutex::new(Inner {
                settings,
                running: None,
                error: note,
            }),
            on_change: None,
        }
    }

    /// Calls `listener` with the new status after every start, stop or
    /// settings change, so every window shows the same state.
    #[must_use]
    pub fn with_listener(mut self, listener: StatusListener) -> Self {
        self.on_change = Some(listener);
        self
    }

    fn changed(&self, status: &McpStatus) {
        if let Some(listener) = &self.on_change {
            listener(status.clone());
        }
    }

    /// Current settings and server state.
    pub async fn status(&self) -> McpStatus {
        Self::status_of(&*self.inner.lock().await)
    }

    fn status_of(inner: &Inner) -> McpStatus {
        McpStatus {
            settings: inner.settings.clone(),
            running: inner.running.is_some(),
            url: inner.running.as_ref().map(RunningServer::url),
            lan_url: inner.running.as_ref().and_then(RunningServer::lan_url),
            error: inner.error.clone(),
            stdio_command: crate::cli::launcher(),
        }
    }

    /// Starts the server if the settings say so (at app start).
    pub async fn start_if_enabled(&self) -> McpStatus {
        let mut inner = self.inner.lock().await;
        if inner.settings.enabled && inner.running.is_none() {
            self.start(&mut inner).await;
        }
        let status = Self::status_of(&inner);
        self.changed(&status);
        status
    }

    /// Validates, saves and applies new settings, restarting the server if
    /// it runs. Invalid settings change nothing.
    pub async fn apply(&self, settings: McpSettings) -> Result<McpStatus, String> {
        settings.validate()?;
        if let Some(path) = &self.settings_path {
            settings::save(path, &settings)?;
        }
        let mut inner = self.inner.lock().await;
        Self::stop_running(&mut inner).await;
        self.approvals.set_enabled(settings.confirm);
        inner.settings = settings;
        inner.error = None;
        if inner.settings.enabled {
            self.start(&mut inner).await;
        }
        let status = Self::status_of(&inner);
        self.changed(&status);
        Ok(status)
    }

    /// Replaces the token with a new random one (connected clients must be
    /// reconfigured).
    pub async fn regenerate_token(&self) -> Result<McpStatus, String> {
        let current = self.inner.lock().await.settings.clone();
        let fresh = McpSettings::new_default()?;
        self.apply(McpSettings {
            token: fresh.token,
            ..current
        })
        .await
    }

    /// Writes SerialBot into the configuration file of the client `id`
    /// (`claude-desktop`, `cursor`, `vscode`; see
    /// [`serialbot_mcp::clients`]). HTTP clients get the address the
    /// server runs on (the configured port when it is off) and the token;
    /// Claude Desktop gets `stdio_command` with `mcp`. `packaged` (an MSIX
    /// install) keeps it from creating files Windows would hide from the
    /// client. A file that cannot be changed is reported with the entry to
    /// add by hand; only an unknown id is an error.
    pub async fn install_client(
        &self,
        id: &str,
        dirs: &clients::Dirs,
        stdio_command: Option<String>,
        packaged: bool,
    ) -> Result<ClientInstall, String> {
        let client = Client::parse(id).ok_or_else(|| format!("unknown MCP client {id}"))?;
        let status = self.status().await;
        let server = clients::Server {
            url: status.url.unwrap_or_else(|| {
                format!(
                    "http://127.0.0.1:{}{}",
                    status.settings.port,
                    serialbot_mcp::MCP_PATH
                )
            }),
            token: status.settings.token,
            stdio: stdio_command.map(|c| (c, vec!["mcp".to_owned()])),
        };
        let path = client.config_path(dirs).display().to_string();
        let create = clients::may_create(client, dirs, packaged);
        Ok(match clients::install(client, &server, dirs, create) {
            Ok(done) => ClientInstall {
                client: client.name().to_owned(),
                path,
                backup: done.backup.map(|b| b.display().to_string()),
                changed: done.changed,
                error: None,
                snippet: None,
            },
            Err(e) => ClientInstall {
                client: client.name().to_owned(),
                path,
                backup: None,
                changed: false,
                error: Some(e.to_string()),
                snippet: client.snippet(&server).ok(),
            },
        })
    }

    /// Stops the server (at app exit). Settings are unchanged.
    pub async fn stop(&self) {
        Self::stop_running(&mut *self.inner.lock().await).await;
    }

    async fn start(&self, inner: &mut Inner) {
        let guard: Arc<dyn serialbot_mcp::guard::Guard> = Arc::clone(&self.approvals) as _;
        let tools = SerialTools::new(Arc::clone(&self.manager), Arc::clone(&self.list_ports))
            .with_guard(guard);
        let settings = &inner.settings;
        match RunningServer::start(
            tools,
            settings.token.clone(),
            settings.listen,
            settings.port,
        )
        .await
        {
            Ok(server) => {
                inner.running = Some(server);
                inner.error = None;
            }
            Err(e) => inner.error = Some(e),
        }
    }

    async fn stop_running(inner: &mut Inner) {
        if let Some(server) = inner.running.take() {
            server.stop().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rmcp::ServiceExt;
    use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
    use rmcp::service::{RoleClient, RunningService};
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
    use serde_json::json;
    use serialbot_core::transport::TransportError;
    use serialbot_core::transport::mock::{self, MockDevice};
    use serialbot_core::{SessionOptions, SystemClock};

    use crate::approval::{ApprovalRequest, Decision};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;
    use crate::settings::Listen;
    use crate::settings::tests::TempDir;
    use tokio::net::TcpListener;

    const TOKEN: &str = "controller-test-token";

    fn controller(path: Option<PathBuf>) -> McpController {
        let manager = SessionManager::new(
            Box::new(|_: &serialbot_core::transport::serial::SerialConfig| {
                Err(TransportError::NotFound("none".into()))
            }),
            Arc::new(SystemClock),
            SessionOptions::default(),
        );
        McpController::new(
            Arc::new(manager),
            Arc::new(|| Ok(Vec::new())),
            approvals(),
            McpSettings {
                enabled: false,
                port: 0,
                token: TOKEN.into(),
                confirm: true,
                listen: Listen::Local,
            },
            path,
            None,
        )
    }

    fn approvals() -> Arc<Approvals> {
        Arc::new(Approvals::new(Duration::from_secs(1), Box::new(|_| {})))
    }

    /// A free port with the next `PORT_ATTEMPTS` ports likely free too.
    async fn free_port() -> u16 {
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        probe.local_addr().unwrap().port()
    }

    fn port_of(status: &McpStatus) -> u16 {
        status
            .url
            .as_deref()
            .unwrap()
            .trim_start_matches("http://127.0.0.1:")
            .trim_end_matches("/mcp")
            .parse()
            .unwrap()
    }

    /// Status code of an authenticated MCP `initialize` POST.
    async fn initialize_status(port: u16, token: &str) -> u16 {
        initialize_status_via(port, token, "127.0.0.1").await
    }

    /// Like [`initialize_status`], with this `Host` header.
    async fn initialize_status_via(port: u16, token: &str, host: &str) -> u16 {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHost: {host}:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut head = vec![0u8; 32];
        let n = stream.read(&mut head).await.unwrap();
        String::from_utf8_lossy(&head[..n])
            .split(' ')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap()
    }

    fn enabled(port: u16, token: &str) -> McpSettings {
        McpSettings {
            enabled: true,
            port,
            token: token.into(),
            confirm: true,
            listen: Listen::Local,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn adds_itself_to_mcp_clients() {
        let dir = TempDir::new("install-clients");
        let dirs = clients::Dirs {
            config: dir.0.join("config"),
            home: dir.0.join("home"),
        };
        let c = controller(None);
        let token = c.status().await.settings.token;
        let port = c.status().await.settings.port;

        // Off: the configured port is used.
        let cursor = c
            .install_client("cursor", &dirs, None, false)
            .await
            .unwrap();
        assert_eq!(cursor.client, "Cursor");
        assert!(cursor.changed && cursor.error.is_none(), "{cursor:?}");
        let text = std::fs::read_to_string(&cursor.path).unwrap();
        assert!(
            text.contains(&format!("http://127.0.0.1:{port}/mcp")),
            "{text}"
        );
        assert!(text.contains(&format!("Bearer {token}")));
        assert!(
            !c.install_client("cursor", &dirs, None, false)
                .await
                .unwrap()
                .changed
        );

        // Claude Desktop needs the launcher.
        let none = c
            .install_client("claude-desktop", &dirs, None, false)
            .await
            .unwrap();
        assert!(none.error.is_some() && none.snippet.is_none(), "{none:?}");
        let desktop = c
            .install_client(
                "claude-desktop",
                &dirs,
                Some("/opt/SerialBot".into()),
                false,
            )
            .await
            .unwrap();
        assert!(desktop.changed, "{desktop:?}");
        assert!(
            std::fs::read_to_string(&desktop.path)
                .unwrap()
                .contains("/opt/SerialBot")
        );

        // Packaged: no new file under the config directory, the entry to paste instead.
        let vscode = c.install_client("vscode", &dirs, None, true).await.unwrap();
        assert!(!vscode.changed && vscode.error.is_some(), "{vscode:?}");
        assert!(vscode.snippet.unwrap().contains("\"servers\""));
        assert!(!std::path::Path::new(&vscode.path).exists());

        assert!(c.install_client("zed", &dirs, None, false).await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn starts_serves_and_stops() {
        let c = controller(None);
        let status = c.start_if_enabled().await;
        assert!(!status.running, "disabled by default");
        assert_eq!(status.url, None);

        let port = free_port().await;
        let status = c.apply(enabled(port, TOKEN)).await.unwrap();
        assert!(status.running);
        assert_eq!(status.url, Some(format!("http://127.0.0.1:{port}/mcp")));
        assert_eq!(status.error, None);
        assert_eq!(initialize_status(port, TOKEN).await, 200);
        assert_eq!(initialize_status(port, "wrong-token-00000000").await, 401);

        c.stop().await;
        assert!(!c.status().await.running);
        assert!(
            tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_err()
        );
        // Starting again after a stop works.
        assert!(c.start_if_enabled().await.running);
        c.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn tells_the_listener_about_every_change() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::<McpStatus>::new()));
        let sink = Arc::clone(&seen);
        let c = controller(None).with_listener(Box::new(move |status| {
            sink.lock().unwrap().push(status);
        }));
        let started = c.start_if_enabled().await;
        let port = free_port().await;
        let on = c.apply(enabled(port, TOKEN)).await.unwrap();
        let fresh = c.regenerate_token().await.unwrap();
        assert!(c.apply(enabled(port, "short")).await.is_err());
        let off = c
            .apply(McpSettings {
                enabled: false,
                ..enabled(port, TOKEN)
            })
            .await
            .unwrap();
        // Each change reported once, with what the caller got; a refused
        // change reports nothing.
        assert_eq!(*seen.lock().unwrap(), vec![started, on, fresh, off]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn listens_on_this_computer_or_the_network() {
        let c = controller(None);
        let port = free_port().await;
        // This computer only: other host names are refused (DNS rebinding).
        let local = c.apply(enabled(port, TOKEN)).await.unwrap();
        assert_eq!(local.lan_url, None);
        assert_eq!(initialize_status(port, TOKEN).await, 200);
        assert_ne!(
            initialize_status_via(port, TOKEN, "192.168.1.50").await,
            200
        );
        // Not reachable through the network address.
        if let Some(ip) = lan_address() {
            assert!(tokio::net::TcpStream::connect((ip, port)).await.is_err());
        }

        // The local network: bound to every address, any host name works,
        // and the token is still required.
        let lan = c
            .apply(McpSettings {
                listen: Listen::Lan,
                ..enabled(port, TOKEN)
            })
            .await
            .unwrap();
        assert!(lan.running);
        assert_eq!(lan.url, Some(format!("http://127.0.0.1:{port}/mcp")));
        assert_eq!(
            initialize_status_via(port, TOKEN, "192.168.1.50").await,
            200
        );
        assert_eq!(initialize_status_via(port, TOKEN, "my-pc.local").await, 200);
        assert_eq!(
            initialize_status_via(port, "wrong-token-00000000", "my-pc.local").await,
            401
        );
        // The LAN URL names this computer's address when there is a network.
        match lan_address() {
            Some(ip) => {
                assert_eq!(lan.lan_url, Some(format!("http://{ip}:{port}/mcp")));
                // Reachable through the network address now.
                assert!(tokio::net::TcpStream::connect((ip, port)).await.is_ok());
            }
            None => assert_eq!(lan.lan_url, None),
        }
        c.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn falls_back_to_the_next_free_port() {
        let taken = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = taken.local_addr().unwrap().port();
        let c = controller(None);
        let status = c.apply(enabled(port, TOKEN)).await.unwrap();
        assert!(status.running);
        let actual = port_of(&status);
        assert!(actual > port && actual < port + PORT_ATTEMPTS, "{actual}");
        assert_eq!(status.settings.port, port, "the preference is kept");
        c.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reports_when_no_port_is_free() {
        let c = controller(None);
        // Ports near the top of the range: the attempts run out of numbers.
        let status = c.apply(enabled(u16::MAX, TOKEN)).await.unwrap();
        if status.running {
            c.stop().await;
            return; // 65535 happened to be free; nothing to check.
        }
        assert!(status.error.unwrap().contains("in use"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn apply_restarts_with_new_settings_and_persists() {
        let dir = TempDir::new("controller");
        let path = dir.0.join("mcp.json");
        let c = controller(Some(path.clone()));
        let port = free_port().await;
        c.apply(enabled(port, TOKEN)).await.unwrap();

        let status = c.regenerate_token().await.unwrap();
        let token = status.settings.token.clone();
        assert_ne!(token, TOKEN);
        assert_eq!(initialize_status(port_of(&status), TOKEN).await, 401);
        assert_eq!(initialize_status(port_of(&status), &token).await, 200);
        assert_eq!(settings::load(&path).unwrap().0.token, token);

        let off = c
            .apply(McpSettings {
                enabled: false,
                ..status.settings
            })
            .await
            .unwrap();
        assert!(!off.running);
        assert!(!settings::load(&path).unwrap().0.enabled);
        assert!(c.approvals.enabled());
        c.apply(McpSettings {
            confirm: false,
            ..off.settings
        })
        .await
        .unwrap();
        assert!(!c.approvals.enabled(), "agents' changes go ahead unasked");
        assert!(!settings::load(&path).unwrap().0.confirm);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn invalid_settings_change_nothing() {
        let c = controller(None);
        let port = free_port().await;
        c.apply(enabled(port, TOKEN)).await.unwrap();
        assert!(c.apply(enabled(port, "short")).await.is_err());
        assert!(c.apply(enabled(0, TOKEN)).await.is_err());
        let status = c.status().await;
        assert!(status.running);
        assert_eq!(status.settings.token, TOKEN);
        c.stop().await;
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn notes_and_invalid_tokens_are_reported() {
        let mut c = controller(None);
        c = McpController::new(
            Arc::clone(&c.manager),
            Arc::clone(&c.list_ports),
            Arc::clone(&c.approvals),
            McpSettings {
                confirm: false,
                ..enabled(9, "bad")
            },
            None,
            Some("ignored invalid MCP settings".into()),
        );
        assert_eq!(
            c.status().await.error.as_deref(),
            Some("ignored invalid MCP settings")
        );
        let status = c.start_if_enabled().await;
        assert!(!status.running);
        assert!(status.error.unwrap().contains("at least"));
        assert!(format!("{c:?}").contains("McpController"));
        assert!(!c.approvals.enabled(), "the loaded setting applies at once");
    }

    type Devices = Arc<std::sync::Mutex<std::collections::HashMap<String, MockDevice>>>;

    /// A manager whose `mock*` ports open as mock devices.
    fn mock_manager() -> (Arc<SessionManager>, Devices) {
        let devices: Devices = Arc::default();
        let registry = Arc::clone(&devices);
        let opener = move |config: &serialbot_core::transport::serial::SerialConfig| {
            if !config.path.starts_with("mock") {
                return Err(TransportError::NotFound(config.path.clone()));
            }
            let (connection, device) = mock::connection(&config.path, Duration::from_millis(5));
            registry.lock().unwrap().insert(config.path.clone(), device);
            Ok(connection)
        };
        let manager = SessionManager::new(
            Box::new(opener),
            Arc::new(SystemClock),
            SessionOptions::default(),
        );
        (Arc::new(manager), devices)
    }

    /// Starts a tool call in the background, as an agent waiting for the
    /// user does.
    fn start_call(
        client: &RunningService<RoleClient, ClientConfig>,
        tool: &'static str,
        args: serde_json::Value,
    ) -> tokio::task::JoinHandle<CallToolResult> {
        let peer = client.peer().clone();
        let serde_json::Value::Object(args) = args else {
            panic!("arguments must be an object");
        };
        tokio::spawn(async move {
            peer.call_tool(CallToolRequestParams::new(tool).with_arguments(args))
                .await
                .unwrap()
        })
    }

    /// The request the user is asked about next.
    async fn next_request(approvals: &Approvals) -> ApprovalRequest {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(request) = approvals.pending().into_iter().next() {
                return request;
            }
            assert!(tokio::time::Instant::now() < deadline, "nothing was asked");
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    /// Answers the next request with `decision` and returns the tool result.
    async fn answer(
        approvals: &Approvals,
        call: tokio::task::JoinHandle<CallToolResult>,
        decision: Decision,
    ) -> (ApprovalRequest, CallToolResult) {
        let request = next_request(approvals).await;
        approvals.answer(request.id, decision).unwrap();
        (request, call.await.unwrap())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn agent_actions_wait_for_the_users_answer() {
        let (manager, devices) = mock_manager();
        let approvals = Arc::new(Approvals::new(Duration::from_secs(10), Box::new(|_| {})));
        let c = McpController::new(
            Arc::clone(&manager),
            Arc::new(|| Ok(Vec::new())),
            Arc::clone(&approvals),
            enabled(free_port().await, TOKEN),
            None,
            None,
        );
        let port = port_of(&c.start_if_enabled().await);
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(format!("http://127.0.0.1:{port}/mcp"))
                .auth_header(TOKEN),
        );
        let client = ClientConfig::default().serve(transport).await.unwrap();

        // Opening a port is asked about before it happens.
        let open = start_call(&client, "session.open", json!({"port": "mock0"}));
        let (request, opened) = answer(&approvals, open, Decision::Once).await;
        assert_eq!(
            (
                request.tool.as_str(),
                request.session,
                request.port.as_str()
            ),
            ("session.open", None, "mock0")
        );
        assert_ne!(opened.is_error, Some(true), "{opened:?}");
        let device = devices.lock().unwrap()["mock0"].clone();

        // Allowed once: written; declined: nothing written, the agent is told.
        let write = start_call(&client, "io.write", json!({"session": "s1", "text": "AT"}));
        let (request, written) = answer(&approvals, write, Decision::Once).await;
        assert_eq!(
            (request.tool.as_str(), request.session),
            ("io.write", Some(1))
        );
        assert!(request.detail.contains("AT"), "{request:?}");
        assert_ne!(written.is_error, Some(true), "{written:?}");
        let write = start_call(&client, "io.write", json!({"session": "s1", "text": "no"}));
        let (_, declined) = answer(&approvals, write, Decision::Deny).await;
        assert_eq!(declined.is_error, Some(true));
        let text = &declined.content[0].as_text().unwrap().text;
        assert!(text.contains("the user declined io.write"), "{text}");
        assert_eq!(
            device.take_written(),
            b"AT",
            "a declined write sends nothing"
        );

        // Trusting the session lets later actions on it through unasked.
        let write = start_call(&client, "io.write", json!({"session": "s1", "text": "1"}));
        answer(&approvals, write, Decision::Session).await;
        let again = start_call(&client, "io.write", json!({"session": "s1", "text": "2"}));
        assert_ne!(again.await.unwrap().is_error, Some(true));
        assert!(approvals.pending().is_empty());
        assert_eq!(device.take_written(), b"12");

        client.cancel().await.unwrap();
        c.stop().await;
        manager.close_all();
    }
}
