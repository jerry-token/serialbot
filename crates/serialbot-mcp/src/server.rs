// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Running the MCP server on a TCP port, for any front end (the desktop
//! app, the command-line tool).
//!
//! [`RunningServer::start`] binds the preferred port or, when it is taken,
//! one of the next [`PORT_ATTEMPTS`] ports, on this computer only or on
//! every address ([`Listen`]). [`RunningServer::stop`] lets open
//! connections finish for a moment, then ends the server.

use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::{MCP_PATH, McpOptions, SerialTools};

/// How many ports, starting with the preferred one, are tried.
pub const PORT_ATTEMPTS: u16 = 10;

/// How long stopping waits for open connections to finish.
const STOP_GRACE: Duration = Duration::from_secs(2);

/// The address the MCP server listens on. Saved in settings files as
/// `"local"` or `"lan"`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Listen {
    /// `127.0.0.1`: only programs on this computer.
    #[default]
    Local,
    /// `0.0.0.0`: also other computers on the network. Anyone who can reach
    /// the port and has the token can drive the ports; HTTP is not
    /// encrypted.
    Lan,
}

impl Listen {
    /// The address to bind.
    pub fn address(self) -> Ipv4Addr {
        match self {
            Self::Local => Ipv4Addr::LOCALHOST,
            Self::Lan => Ipv4Addr::UNSPECIFIED,
        }
    }
}

/// This computer's IPv4 address on the network its default route uses, or
/// `None` without one. Asks the OS which local address a UDP "connection"
/// would use; no packet is sent.
pub fn lan_address() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    // A documentation address (TEST-NET-1): only the route is looked up.
    socket.connect(("192.0.2.1", 9)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_unspecified() => Some(ip),
        _ => None,
    }
}

/// Binds `listen` on `first` or one of the following ports.
async fn bind_from(listen: Listen, first: u16) -> Option<TcpListener> {
    for offset in 0..PORT_ATTEMPTS {
        let Some(port) = first.checked_add(offset) else {
            break;
        };
        if let Ok(listener) = TcpListener::bind((listen.address(), port)).await {
            return Some(listener);
        }
    }
    None
}

/// An MCP server serving on a port until [`RunningServer::stop`].
#[derive(Debug)]
pub struct RunningServer {
    port: u16,
    listen: Listen,
    shutdown: CancellationToken,
    task: JoinHandle<std::io::Result<()>>,
}

impl RunningServer {
    /// Serves `tools` at [`MCP_PATH`] with bearer `token`, on the first free
    /// port from `first_port`. On the local network any `Host` name is
    /// accepted (other computers use this computer's address or name); on
    /// this computer only, just `localhost` names are.
    ///
    /// Errors, as messages for the user: the token is too short, or none
    /// of the [`PORT_ATTEMPTS`] ports is free (`"ports A-B are all in
    /// use"`).
    pub async fn start(
        tools: SerialTools,
        token: String,
        listen: Listen,
        first_port: u16,
    ) -> Result<Self, String> {
        let options = McpOptions::new(token).map_err(|e| e.to_string())?;
        let options = match listen {
            Listen::Lan => options.with_any_host(),
            Listen::Local => options,
        };
        let Some(listener) = bind_from(listen, first_port).await else {
            return Err(format!(
                "ports {first_port}-{} are all in use",
                first_port.saturating_add(PORT_ATTEMPTS - 1)
            ));
        };
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let shutdown = CancellationToken::new();
        let app = crate::router(tools, &options, &shutdown);
        let task = tokio::spawn(crate::serve(listener, app, shutdown.clone()));
        Ok(Self {
            port,
            listen,
            shutdown,
            task,
        })
    }

    /// The port actually bound (may be after the preferred one).
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Endpoint URL on this computer, `http://127.0.0.1:<port>/mcp`.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}{MCP_PATH}", self.port)
    }

    /// Endpoint URL for other computers: `None` on this computer only, or
    /// when this computer has no network address ([`lan_address`]).
    pub fn lan_url(&self) -> Option<String> {
        if self.listen != Listen::Lan {
            return None;
        }
        lan_address().map(|ip| format!("http://{ip}:{}{MCP_PATH}", self.port))
    }

    /// Stops accepting requests and ends the server, waiting up to two
    /// seconds for open connections; the port is free when this returns.
    pub async fn stop(self) {
        self.shutdown.cancel();
        let abort = self.task.abort_handle();
        if tokio::time::timeout(STOP_GRACE, self.task).await.is_err() {
            abort.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serialbot_core::transport::TransportError;
    use serialbot_core::{SessionManager, SessionOptions, SystemClock};

    use super::*;

    const TOKEN: &str = "server-test-token-0123";

    fn tools() -> SerialTools {
        let manager = SessionManager::new(
            Box::new(|_: &serialbot_core::transport::serial::SerialConfig| {
                Err(TransportError::NotFound("none".into()))
            }),
            Arc::new(SystemClock),
            SessionOptions::default(),
        );
        SerialTools::new(Arc::new(manager), Arc::new(|| Ok(Vec::new())))
    }

    async fn free_port() -> u16 {
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        probe.local_addr().unwrap().port()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn serves_on_the_port_until_stopped() {
        let port = free_port().await;
        let server = RunningServer::start(tools(), TOKEN.into(), Listen::Local, port)
            .await
            .unwrap();
        assert_eq!(server.port(), port);
        assert_eq!(server.url(), format!("http://127.0.0.1:{port}/mcp"));
        assert_eq!(server.lan_url(), None);
        assert!(
            tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
        );
        assert!(format!("{server:?}").contains("RunningServer"));
        server.stop().await;
        assert!(
            tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_err()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn takes_the_next_port_when_the_first_is_taken() {
        let taken = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = taken.local_addr().unwrap().port();
        let server = RunningServer::start(tools(), TOKEN.into(), Listen::Local, port)
            .await
            .unwrap();
        assert!(server.port() > port && server.port() < port + PORT_ATTEMPTS);
        server.stop().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refuses_short_tokens_and_full_port_ranges() {
        let short = RunningServer::start(tools(), "short".into(), Listen::Local, 0).await;
        assert!(short.unwrap_err().contains("at least"));
        // 65535 is the last port: if it is taken, nothing is left to try.
        let Ok(_top) = TcpListener::bind("127.0.0.1:65535").await else {
            return; // in use by something else already; covered either way
        };
        let full = RunningServer::start(tools(), TOKEN.into(), Listen::Local, u16::MAX).await;
        assert_eq!(full.unwrap_err(), "ports 65535-65535 are all in use");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn on_the_network_it_names_this_computers_address() {
        let port = free_port().await;
        let server = RunningServer::start(tools(), TOKEN.into(), Listen::Lan, port)
            .await
            .unwrap();
        match lan_address() {
            Some(ip) => assert_eq!(server.lan_url(), Some(format!("http://{ip}:{port}/mcp"))),
            None => assert_eq!(server.lan_url(), None),
        }
        server.stop().await;
    }

    #[test]
    fn listen_is_saved_in_lowercase() {
        assert_eq!(Listen::default(), Listen::Local);
        assert_eq!(serde_json::to_string(&Listen::Lan).unwrap(), "\"lan\"");
        assert_eq!(
            serde_json::from_str::<Listen>("\"local\"").unwrap(),
            Listen::Local
        );
        assert_eq!(Listen::Local.address(), Ipv4Addr::LOCALHOST);
        assert_eq!(Listen::Lan.address(), Ipv4Addr::UNSPECIFIED);
    }
}
