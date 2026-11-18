//! MCP server for serialbot.
//!
//! Serves the curated tool set of [`SerialTools`] over MCP Streamable HTTP
//! at `/mcp`. Every request must carry `Authorization: Bearer <token>`;
//! the `Host` header must name an allowed host (loopback by default, which
//! blocks DNS-rebinding attacks), and any request with an `Origin` header
//! is rejected unless that origin is allowed (none are by default), so web
//! pages in a browser cannot call the server.
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use std::sync::Arc;
//! use serialbot_core::SessionManager;
//! use serialbot_mcp::{McpOptions, SerialTools, generate_token, router, serve};
//! use tokio_util::sync::CancellationToken;
//!
//! let manager = Arc::new(SessionManager::with_serial_ports());
//! let tools = SerialTools::new(manager, Arc::new(serialbot_core::transport::serial::list_ports));
//! let options = McpOptions::new(generate_token()?)?;
//! let shutdown = CancellationToken::new();
//! let app = router(tools, &options, &shutdown);
//! let listener = tokio::net::TcpListener::bind("127.0.0.1:9633").await?;
//! serve(listener, app, shutdown).await?;
//! # Ok(()) }
//! ```

mod auth;
mod payload;
mod tools;

use std::sync::Arc;

use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio_util::sync::CancellationToken;

pub use auth::generate_token;
pub use payload::Format;
pub use tools::{
    DEFAULT_MATCH_WINDOW, DEFAULT_READ_BYTES, DEFAULT_TIMEOUT_MS, MAX_READ_BYTES, MAX_TIMEOUT_MS,
    PortLister, SerialTools,
};

/// Path the MCP endpoint is served at.
pub const MCP_PATH: &str = "/mcp";

/// Default TCP port of the MCP server.
pub const DEFAULT_PORT: u16 = 9633;

/// Shortest bearer token [`McpOptions::new`] accepts.
pub const MIN_TOKEN_LEN: usize = 16;

/// Invalid server options.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OptionsError {
    /// The token is shorter than [`MIN_TOKEN_LEN`].
    #[error("the token must be at least {MIN_TOKEN_LEN} characters")]
    TokenTooShort,
    /// The token contains characters that cannot appear in an HTTP header
    /// unescaped (only visible ASCII is allowed).
    #[error("the token may only contain visible ASCII characters")]
    TokenCharacters,
}

/// Security settings of the server. `Debug` output hides the token.
#[derive(Clone)]
pub struct McpOptions {
    token: Arc<str>,
    allowed_hosts: Vec<String>,
}

impl std::fmt::Debug for McpOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpOptions")
            .field("token", &"<redacted>")
            .field("allowed_hosts", &self.allowed_hosts)
            .finish()
    }
}

impl McpOptions {
    /// Options requiring `token`, accepting only loopback `Host` names
    /// (`localhost`, `127.0.0.1`, `::1`, any port).
    ///
    /// The token must be at least [`MIN_TOKEN_LEN`] visible ASCII
    /// characters.
    pub fn new(token: impl Into<String>) -> Result<Self, OptionsError> {
        let token = token.into();
        if token.len() < MIN_TOKEN_LEN {
            return Err(OptionsError::TokenTooShort);
        }
        if !token.bytes().all(|c| c.is_ascii_graphic()) {
            return Err(OptionsError::TokenCharacters);
        }
        Ok(Self {
            token: token.into(),
            allowed_hosts: vec!["localhost".into(), "127.0.0.1".into(), "::1".into()],
        })
    }

    /// Replaces the accepted `Host` names, for example to add a LAN address.
    /// Entries are host names or `host:port` authorities.
    #[must_use]
    pub fn with_allowed_hosts(
        mut self,
        hosts: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.allowed_hosts = hosts.into_iter().map(Into::into).collect();
        self
    }

    /// The bearer token clients must send.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The accepted `Host` names.
    pub fn allowed_hosts(&self) -> &[String] {
        &self.allowed_hosts
    }
}

/// Builds the HTTP app serving `tools` at [`MCP_PATH`].
///
/// Cancelling `shutdown` ends all MCP sessions; pass the same token to
/// [`serve`] to stop accepting connections too.
pub fn router(
    tools: SerialTools,
    options: &McpOptions,
    shutdown: &CancellationToken,
) -> axum::Router {
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(options.allowed_hosts.clone())
        .enforce_origin_validation()
        .with_cancellation_token(shutdown.child_token());
    let service = StreamableHttpService::new(
        move || Ok(tools.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    axum::Router::new()
        .nest_service(MCP_PATH, service)
        .layer(axum::middleware::from_fn_with_state(
            Arc::clone(&options.token),
            auth::require_token,
        ))
}

/// Serves `app` on `listener` until `shutdown` is cancelled.
pub async fn serve(
    listener: tokio::net::TcpListener,
    app: axum::Router,
    shutdown: CancellationToken,
) -> std::io::Result<()> {
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown.cancelled_owned())
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_validate_the_token() {
        assert_eq!(
            McpOptions::new("short").unwrap_err(),
            OptionsError::TokenTooShort
        );
        assert_eq!(
            McpOptions::new("sixteen chars ok").unwrap_err(),
            OptionsError::TokenCharacters
        );
        assert_eq!(
            McpOptions::new("0123456789abcdé").unwrap_err(),
            OptionsError::TokenCharacters
        );
        let options = McpOptions::new("0123456789abcdef").unwrap();
        assert_eq!(options.token(), "0123456789abcdef");
        assert_eq!(options.allowed_hosts(), ["localhost", "127.0.0.1", "::1"]);
        assert!(!format!("{options:?}").contains("0123456789abcdef"));
        let options = options.with_allowed_hosts(["192.168.1.5"]);
        assert_eq!(options.allowed_hosts(), ["192.168.1.5"]);
    }
}
