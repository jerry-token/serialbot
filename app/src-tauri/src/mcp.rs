//! The MCP server embedded in the app.
//!
//! It serves the same [`SessionManager`] as the window, so a port opened in
//! the UI can be driven by an agent and everything the agent sends shows up
//! in the UI. It listens on 127.0.0.1 only.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serialbot_core::SessionManager;
use serialbot_mcp::{MCP_PATH, McpOptions, PortLister, SerialTools};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::settings::{self, McpSettings};

/// How many ports after the preferred one are tried when it is taken.
pub const PORT_ATTEMPTS: u16 = 10;

/// How long stopping waits for open connections to finish.
const STOP_GRACE: Duration = Duration::from_secs(2);

/// What the UI shows about the MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    /// The user's settings.
    pub settings: McpSettings,
    /// Whether the server is listening.
    pub running: bool,
    /// Endpoint URL while running.
    pub url: Option<String>,
    /// Why the server is not running although enabled, or why the settings
    /// file was ignored.
    pub error: Option<String>,
}

struct Running {
    port: u16,
    shutdown: CancellationToken,
    task: JoinHandle<std::io::Result<()>>,
}

struct Inner {
    settings: McpSettings,
    running: Option<Running>,
    error: Option<String>,
}

/// Starts, stops and reconfigures the embedded server.
pub struct McpController {
    manager: Arc<SessionManager>,
    list_ports: Arc<PortLister>,
    /// Where settings are saved; `None` keeps them in memory (tests).
    settings_path: Option<PathBuf>,
    inner: Mutex<Inner>,
}

impl std::fmt::Debug for McpController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpController")
            .field("settings_path", &self.settings_path)
            .finish_non_exhaustive()
    }
}

impl McpController {
    /// A stopped controller. `note` is shown as the error until the next
    /// change (for example why the settings file was ignored).
    pub fn new(
        manager: Arc<SessionManager>,
        list_ports: Arc<PortLister>,
        settings: McpSettings,
        settings_path: Option<PathBuf>,
        note: Option<String>,
    ) -> Self {
        Self {
            manager,
            list_ports,
            settings_path,
            inner: Mutex::new(Inner {
                settings,
                running: None,
                error: note,
            }),
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
            url: inner
                .running
                .as_ref()
                .map(|r| format!("http://127.0.0.1:{}{MCP_PATH}", r.port)),
            error: inner.error.clone(),
        }
    }

    /// Starts the server if the settings say so (at app start).
    pub async fn start_if_enabled(&self) -> McpStatus {
        let mut inner = self.inner.lock().await;
        if inner.settings.enabled && inner.running.is_none() {
            self.start(&mut inner).await;
        }
        Self::status_of(&inner)
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
        inner.settings = settings;
        inner.error = None;
        if inner.settings.enabled {
            self.start(&mut inner).await;
        }
        Ok(Self::status_of(&inner))
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

    /// Stops the server (at app exit). Settings are unchanged.
    pub async fn stop(&self) {
        Self::stop_running(&mut *self.inner.lock().await).await;
    }

    async fn start(&self, inner: &mut Inner) {
        let options = match McpOptions::new(inner.settings.token.clone()) {
            Ok(options) => options,
            Err(e) => {
                inner.error = Some(e.to_string());
                return;
            }
        };
        let Some(listener) = bind_from(inner.settings.port).await else {
            inner.error = Some(format!(
                "ports {}-{} are all in use",
                inner.settings.port,
                inner.settings.port.saturating_add(PORT_ATTEMPTS - 1)
            ));
            return;
        };
        let port = match listener.local_addr() {
            Ok(addr) => addr.port(),
            Err(e) => {
                inner.error = Some(e.to_string());
                return;
            }
        };
        let shutdown = CancellationToken::new();
        let tools = SerialTools::new(Arc::clone(&self.manager), Arc::clone(&self.list_ports));
        let app = serialbot_mcp::router(tools, &options, &shutdown);
        let task = tokio::spawn(serialbot_mcp::serve(listener, app, shutdown.clone()));
        inner.running = Some(Running {
            port,
            shutdown,
            task,
        });
        inner.error = None;
    }

    async fn stop_running(inner: &mut Inner) {
        if let Some(running) = inner.running.take() {
            running.shutdown.cancel();
            let abort = running.task.abort_handle();
            if tokio::time::timeout(STOP_GRACE, running.task)
                .await
                .is_err()
            {
                abort.abort();
            }
        }
    }
}

/// Binds 127.0.0.1 on `first` or one of the following ports.
async fn bind_from(first: u16) -> Option<TcpListener> {
    for offset in 0..PORT_ATTEMPTS {
        let Some(port) = first.checked_add(offset) else {
            break;
        };
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)).await {
            return Some(listener);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serialbot_core::transport::TransportError;
    use serialbot_core::{SessionOptions, SystemClock};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;
    use crate::settings::tests::TempDir;

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
            McpSettings {
                enabled: false,
                port: 0,
                token: TOKEN.into(),
            },
            path,
            None,
        )
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
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
        let request = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
        }
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
            enabled(9, "bad"),
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
    }
}
