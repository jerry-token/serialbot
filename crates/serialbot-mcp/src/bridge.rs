// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! A stdio MCP server that forwards to SerialBot's HTTP MCP server.
//!
//! Some clients (Claude Desktop, for one) only start MCP servers as child
//! processes speaking JSON-RPC over stdio. The serial ports must stay owned
//! by the running app, so the bridge owns nothing: it connects to the app's
//! server on 127.0.0.1 and passes tool listings and calls through.

use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ClientConfig, Implementation, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig,
};
use rmcp::service::{Peer, RequestContext, RoleClient, RoleServer, RunningService, ServiceError};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::{ErrorData, ServerHandler, ServiceExt};

/// How long one connection attempt to the app may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// Why the bridge could not start or stopped.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// No candidate URL answered as an MCP server accepting the token.
    #[error(
        "cannot reach SerialBot's MCP server (tried {tried}): start SerialBot and turn the MCP server on in its AI page"
    )]
    Unreachable {
        /// The URLs tried, comma-separated.
        tried: String,
    },
    /// The stdio side failed.
    #[error("stdio: {0}")]
    Stdio(String),
}

/// Forwards tool calls to the app's server.
#[derive(Debug, Clone)]
pub struct Bridge {
    upstream: Peer<RoleClient>,
    info: ServerConfig,
}

fn upstream_error(e: ServiceError) -> ErrorData {
    match e {
        ServiceError::McpError(e) => e,
        other => ErrorData::internal_error(format!("SerialBot did not answer: {other}"), None),
    }
}

impl ServerHandler for Bridge {
    fn get_info(&self) -> ServerConfig {
        self.info.clone()
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        self.upstream
            .list_tools(request)
            .await
            .map_err(upstream_error)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.upstream
            .call_tool(request)
            .await
            .map(Into::into)
            .map_err(upstream_error)
    }
}

/// Connects to the first of `urls` that accepts `token` as an MCP server.
///
/// The app may listen on a fallback port, so callers pass every URL it
/// might use. Each attempt gives up after a few seconds.
pub async fn connect(
    urls: &[String],
    token: &str,
) -> Result<RunningService<RoleClient, ClientConfig>, BridgeError> {
    for url in urls {
        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(url.clone()).auth_header(token),
        );
        let client = ClientConfig::new(
            rmcp::model::ClientCapabilities::default(),
            Implementation::new("serialbot-bridge", env!("CARGO_PKG_VERSION")),
        );
        if let Ok(Ok(service)) =
            tokio::time::timeout(CONNECT_TIMEOUT, client.serve(transport)).await
        {
            return Ok(service);
        }
    }
    Err(BridgeError::Unreachable {
        tried: urls.join(", "),
    })
}

impl Bridge {
    /// A bridge over a connected client, presenting the app's own name and
    /// instructions to the stdio client.
    pub fn new(upstream: &RunningService<RoleClient, ClientConfig>) -> Self {
        let peer = upstream.peer_info();
        let server_info = peer
            .as_ref()
            .and_then(|p| p.server_info.clone())
            .unwrap_or_else(|| Implementation::new("serialbot", env!("CARGO_PKG_VERSION")));
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(server_info);
        if let Some(instructions) = peer.as_ref().and_then(|p| p.instructions.clone()) {
            info = info.with_instructions(instructions);
        }
        Self {
            upstream: upstream.peer().clone(),
            info,
        }
    }
}

/// Runs the bridge on this process's stdin/stdout until the client closes
/// it: connects to the app (see [`connect`]), then serves.
pub async fn run_stdio(urls: &[String], token: &str) -> Result<(), BridgeError> {
    let upstream = connect(urls, token).await?;
    let bridge = Bridge::new(&upstream);
    let service = bridge
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| BridgeError::Stdio(e.to_string()))?;
    service
        .waiting()
        .await
        .map_err(|e| BridgeError::Stdio(e.to_string()))?;
    // The upstream session ends with the bridge.
    let _ = upstream.cancel().await;
    Ok(())
}
