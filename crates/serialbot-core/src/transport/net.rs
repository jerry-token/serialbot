// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Network transports: TCP client, TCP server and UDP.
//!
//! A network endpoint is written as a URL in the place of a port path, so
//! everything above the transports (sessions, the manager, reconnecting,
//! MCP) treats it like any other port:
//!
//! - `tcp://host:port`: connect to a TCP server. Reconnecting dials again.
//! - `tcp-server://addr:port`: listen and serve up to [`MAX_SERVER_CLIENTS`]
//!   clients at once. Received data from all of them goes into one stream
//!   (a `data from <peer>` event marks a change of sender); writes go to
//!   every client and fail only while none is connected.
//!   `tcp-server://:9000` listens on all interfaces.
//! - `udp://host:port?local=addr:port`: send datagrams to `host:port` and
//!   receive on `local` (default: any free port on all interfaces). Without
//!   a remote (`udp://?local=0.0.0.0:9000`) replies go to the last sender.
//!
//! Network links have no modem lines, BREAK or baud rate; those calls fail
//! with [`TransportError::Unsupported`]. Client connects, leaves and UDP
//! peers are reported as reader events, which sessions log as markers.

use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use super::{
    Capabilities, Connection, LineStates, TransportControl, TransportError, TransportInfo,
    TransportKind, TransportReader,
};

/// How long dialing a TCP server may take before giving up.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// A network endpoint parsed from a port path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// `tcp://host:port`
    TcpClient {
        /// `host:port` to dial.
        remote: String,
    },
    /// `tcp-server://addr:port`
    TcpServer {
        /// `addr:port` to listen on.
        local: String,
    },
    /// `udp://host:port?local=addr:port`
    Udp {
        /// Where datagrams go; `None` answers the last sender.
        remote: Option<String>,
        /// Where datagrams are received.
        local: String,
    },
}

/// Whether `path` names a network endpoint rather than a serial port.
pub fn is_network(path: &str) -> bool {
    ["tcp://", "tcp-server://", "udp://"]
        .iter()
        .any(|scheme| path.starts_with(scheme))
}

/// Checks that `addr` is `host:port` with a port number; an empty host
/// becomes `0.0.0.0` (all interfaces) when `listen` is set.
fn host_port(addr: &str, listen: bool, path: &str) -> Result<String, TransportError> {
    let bad = || {
        TransportError::InvalidConfig(format!(
            "{path}: expected host:port, for example 192.168.1.10:8080"
        ))
    };
    let (host, port) = addr.rsplit_once(':').ok_or_else(bad)?;
    port.parse::<u16>().map_err(|_| bad())?;
    match (host.is_empty(), listen) {
        (true, true) => Ok(format!("0.0.0.0:{port}")),
        (true, false) => Err(bad()),
        (false, _) => Ok(addr.to_owned()),
    }
}

impl Endpoint {
    /// Parses a network port path. Returns `None` for paths that are not
    /// network URLs (serial ports), and an error for malformed URLs.
    pub fn parse(path: &str) -> Option<Result<Self, TransportError>> {
        if let Some(rest) = path.strip_prefix("tcp://") {
            return Some(host_port(rest, false, path).map(|remote| Self::TcpClient { remote }));
        }
        if let Some(rest) = path.strip_prefix("tcp-server://") {
            return Some(host_port(rest, true, path).map(|local| Self::TcpServer { local }));
        }
        let rest = path.strip_prefix("udp://")?;
        let (remote, query) = rest.split_once('?').unwrap_or((rest, ""));
        let mut local = "0.0.0.0:0".to_owned();
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            match pair.split_once('=') {
                Some(("local", value)) => match host_port(value, true, path) {
                    Ok(addr) => local = addr,
                    Err(e) => return Some(Err(e)),
                },
                _ => {
                    return Some(Err(TransportError::InvalidConfig(format!(
                        "{path}: unknown option {pair:?} (only local=addr:port)"
                    ))));
                }
            }
        }
        let remote = if remote.is_empty() {
            None
        } else {
            match host_port(remote, false, path) {
                Ok(addr) => Some(addr),
                Err(e) => return Some(Err(e)),
            }
        };
        if remote.is_none() && local.ends_with(":0") {
            return Some(Err(TransportError::InvalidConfig(format!(
                "{path}: give a remote host:port, a local port to listen on, or both"
            ))));
        }
        Some(Ok(Self::Udp { remote, local }))
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn resolve(addr: &str) -> Result<Vec<SocketAddr>, TransportError> {
    let addrs: Vec<SocketAddr> = addr
        .to_socket_addrs()
        .map_err(|e| TransportError::NotFound(format!("{addr}: {e}")))?
        .collect();
    if addrs.is_empty() {
        return Err(TransportError::NotFound(format!("{addr}: no address")));
    }
    Ok(addrs)
}

/// Read results that only mean "nothing yet".
fn is_idle(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}

/// Errors that mean the peer is gone.
fn is_gone(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::BrokenPipe
            | ErrorKind::UnexpectedEof
            | ErrorKind::NotConnected
    )
}

fn info(kind: TransportKind, name: &str) -> TransportInfo {
    TransportInfo {
        kind,
        name: name.to_owned(),
        capabilities: Capabilities::default(),
    }
}

/// Opens the network endpoint at `path` (see the module docs for the
/// forms), reading with `read_timeout`.
///
/// Fails with [`TransportError::InvalidConfig`] for a malformed URL,
/// [`TransportError::NotFound`] when a TCP server cannot be reached or a
/// host does not resolve, and [`TransportError::Busy`] when a local
/// address is already in use.
pub fn open(path: &str, read_timeout: Duration) -> Result<Connection, TransportError> {
    let endpoint = Endpoint::parse(path).unwrap_or_else(|| {
        Err(TransportError::InvalidConfig(format!(
            "{path} is not a network address"
        )))
    })?;
    let read_timeout = read_timeout.max(Duration::from_millis(1));
    match endpoint {
        Endpoint::TcpClient { remote } => open_tcp_client(path, &remote, read_timeout),
        Endpoint::TcpServer { local } => open_tcp_server(path, &local, read_timeout),
        Endpoint::Udp { remote, local } => open_udp(path, remote.as_deref(), &local, read_timeout),
    }
}

fn bind_error(local: &str, e: &std::io::Error) -> TransportError {
    if e.kind() == ErrorKind::AddrInUse {
        TransportError::Busy(local.to_owned())
    } else {
        TransportError::Io(std::io::Error::new(e.kind(), format!("{local}: {e}")))
    }
}

// ---- TCP client --------------------------------------------------------

fn open_tcp_client(
    path: &str,
    remote: &str,
    read_timeout: Duration,
) -> Result<Connection, TransportError> {
    let mut last = None;
    for addr in resolve(remote)? {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(stream) => {
                stream.set_read_timeout(Some(read_timeout))?;
                // Small writes (keystrokes, AT commands) go out at once.
                stream.set_nodelay(true)?;
                let reader = stream.try_clone()?;
                return Ok(Connection {
                    info: info(TransportKind::TcpClient, path),
                    reader: Box::new(StreamReader { stream: reader }),
                    control: Box::new(StreamControl {
                        stream: Arc::new(Mutex::new(Some(stream))),
                    }),
                });
            }
            Err(e) => last = Some(e),
        }
    }
    let reason = last.map_or_else(|| "no address".to_owned(), |e| e.to_string());
    Err(TransportError::NotFound(format!("{remote}: {reason}")))
}

struct StreamReader {
    stream: TcpStream,
}

impl TransportReader for StreamReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        match self.stream.read(buf) {
            Ok(0) => Err(TransportError::Disconnected),
            Ok(n) => Ok(n),
            Err(e) if is_idle(e.kind()) => Ok(0),
            Err(e) if is_gone(e.kind()) => Err(TransportError::Disconnected),
            Err(e) => Err(e.into()),
        }
    }
}

/// Write half of a TCP client link.
struct StreamControl {
    stream: Arc<Mutex<Option<TcpStream>>>,
}

impl TransportControl for StreamControl {
    fn write_all(&mut self, data: &[u8]) -> Result<(), TransportError> {
        let mut guard = lock(&self.stream);
        let stream = guard.as_mut().ok_or_else(|| {
            TransportError::Io(std::io::Error::new(
                ErrorKind::NotConnected,
                "no client is connected",
            ))
        })?;
        stream.write_all(data).map_err(|e| {
            if is_gone(e.kind()) {
                TransportError::Disconnected
            } else {
                e.into()
            }
        })
    }

    fn set_dtr(&mut self, _: bool) -> Result<(), TransportError> {
        Err(TransportError::Unsupported("DTR on a network connection"))
    }

    fn set_rts(&mut self, _: bool) -> Result<(), TransportError> {
        Err(TransportError::Unsupported("RTS on a network connection"))
    }

    fn line_states(&mut self) -> Result<LineStates, TransportError> {
        Err(TransportError::Unsupported(
            "modem lines on a network connection",
        ))
    }

    fn send_break(&mut self, _: Duration) -> Result<(), TransportError> {
        Err(TransportError::Unsupported("BREAK on a network connection"))
    }

    fn set_baud_rate(&mut self, _: u32) -> Result<(), TransportError> {
        Err(TransportError::Unsupported(
            "baud rate on a network connection",
        ))
    }
}

// ---- TCP server --------------------------------------------------------

/// Most clients a TCP server session serves at once; later ones are refused.
pub const MAX_SERVER_CLIENTS: usize = 16;

/// How long a server read waits when no client sent anything.
const SERVER_POLL: Duration = Duration::from_millis(5);

fn open_tcp_server(
    path: &str,
    local: &str,
    read_timeout: Duration,
) -> Result<Connection, TransportError> {
    let addr = resolve(local)?[0];
    let listener = TcpListener::bind(addr).map_err(|e| bind_error(local, &e))?;
    listener.set_nonblocking(true)?;
    let writers = Arc::new(Mutex::new(Vec::new()));
    Ok(Connection {
        info: info(TransportKind::TcpServer, path),
        reader: Box::new(ServerReader {
            listener,
            clients: Vec::new(),
            writers: Arc::clone(&writers),
            read_timeout,
            last_sender: None,
            events: Vec::new(),
        }),
        control: Box::new(ServerControl { writers }),
    })
}

/// A connected client: its address and the read half.
struct Client {
    peer: SocketAddr,
    stream: TcpStream,
}

/// Accepts clients and reads from all of them.
///
/// Connects, leaves and refusals are reported as events; when data comes
/// from a different client than the data before it, a `data from <peer>`
/// event says who is talking.
struct ServerReader {
    listener: TcpListener,
    clients: Vec<Client>,
    /// Write halves, shared with the control (which broadcasts).
    writers: Arc<Mutex<Vec<(SocketAddr, TcpStream)>>>,
    read_timeout: Duration,
    last_sender: Option<SocketAddr>,
    events: Vec<String>,
}

impl ServerReader {
    /// Takes every pending connection.
    fn accept_all(&mut self) -> Result<(), TransportError> {
        loop {
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    if self.clients.len() >= MAX_SERVER_CLIENTS {
                        let _ = stream.shutdown(Shutdown::Both);
                        self.events.push(format!(
                            "client {peer} refused (limit of {MAX_SERVER_CLIENTS} clients)"
                        ));
                        continue;
                    }
                    // Non-blocking for both halves: the flag is shared by
                    // clones of a socket.
                    stream.set_nonblocking(true)?;
                    stream.set_nodelay(true)?;
                    lock(&self.writers).push((peer, stream.try_clone()?));
                    self.clients.push(Client { peer, stream });
                    self.events.push(format!("client {peer} connected"));
                }
                Err(e) if is_idle(e.kind()) => return Ok(()),
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Forgets a client that left, and its write half.
    fn drop_client(&mut self, index: usize) {
        let client = self.clients.remove(index);
        let _ = client.stream.shutdown(Shutdown::Both);
        lock(&self.writers).retain(|(peer, _)| *peer != client.peer);
        if self.last_sender == Some(client.peer) {
            self.last_sender = None;
        }
        self.events
            .push(format!("client {} disconnected", client.peer));
    }

    /// Reads what one client has; `None` when it has nothing now.
    fn read_from(&mut self, index: usize, buf: &mut [u8]) -> Result<Option<usize>, TransportError> {
        let Some(client) = self.clients.get_mut(index) else {
            return Ok(None);
        };
        match client.stream.read(buf) {
            Ok(0) => {
                self.drop_client(index);
                Ok(None)
            }
            Ok(n) => {
                let peer = client.peer;
                if self.last_sender != Some(peer) && self.clients.len() > 1 {
                    self.events.push(format!("data from {peer}"));
                }
                self.last_sender = Some(peer);
                Ok(Some(n))
            }
            Err(e) if is_idle(e.kind()) => Ok(None),
            Err(e) if is_gone(e.kind()) => {
                self.drop_client(index);
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }
}

impl TransportReader for ServerReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        self.accept_all()?;
        // Clients are polled in turn; the last sender first, so a burst
        // from one client is not interleaved with others more than needed.
        let start = self
            .last_sender
            .and_then(|p| self.clients.iter().position(|c| c.peer == p))
            .unwrap_or(0);
        let count = self.clients.len();
        for k in 0..count {
            let index = (start + k) % count;
            if index >= self.clients.len() {
                break; // A client left during this pass.
            }
            if let Some(n) = self.read_from(index, buf)? {
                return Ok(n);
            }
        }
        // Nothing from anyone: wait a little, like a read timeout would.
        std::thread::sleep(self.read_timeout.min(SERVER_POLL));
        Ok(0)
    }

    fn take_event(&mut self) -> Option<String> {
        (!self.events.is_empty()).then(|| self.events.remove(0))
    }
}

/// Longest wait for a client that does not take data (full send buffer).
const SERVER_WRITE_PATIENCE: Duration = Duration::from_secs(2);

/// Writes all of `data` to a non-blocking stream, waiting briefly while its
/// send buffer is full; false if it fails or stays full for
/// [`SERVER_WRITE_PATIENCE`].
fn write_patiently(stream: &mut TcpStream, mut data: &[u8]) -> bool {
    let deadline = std::time::Instant::now() + SERVER_WRITE_PATIENCE;
    while !data.is_empty() {
        match stream.write(data) {
            Ok(0) => return false,
            Ok(n) => data = &data[n..],
            Err(e) if is_idle(e.kind()) && std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
    true
}

/// Sends to every connected client.
struct ServerControl {
    writers: Arc<Mutex<Vec<(SocketAddr, TcpStream)>>>,
}

impl TransportControl for ServerControl {
    /// Writes `data` to every client. Fails only when no client is
    /// connected; a client whose write fails is skipped (its reader notices
    /// it is gone and reports the disconnect).
    fn write_all(&mut self, data: &[u8]) -> Result<(), TransportError> {
        let mut writers = lock(&self.writers);
        if writers.is_empty() {
            return Err(TransportError::Io(std::io::Error::new(
                ErrorKind::NotConnected,
                "no client is connected",
            )));
        }
        let mut delivered = 0;
        writers.retain_mut(|(_, stream)| {
            let ok = write_patiently(stream, data);
            delivered += usize::from(ok);
            ok
        });
        if delivered == 0 {
            return Err(TransportError::Disconnected);
        }
        Ok(())
    }

    fn set_dtr(&mut self, _: bool) -> Result<(), TransportError> {
        Err(TransportError::Unsupported("DTR on a network connection"))
    }

    fn set_rts(&mut self, _: bool) -> Result<(), TransportError> {
        Err(TransportError::Unsupported("RTS on a network connection"))
    }

    fn line_states(&mut self) -> Result<LineStates, TransportError> {
        Err(TransportError::Unsupported(
            "modem lines on a network connection",
        ))
    }

    fn send_break(&mut self, _: Duration) -> Result<(), TransportError> {
        Err(TransportError::Unsupported("BREAK on a network connection"))
    }

    fn set_baud_rate(&mut self, _: u32) -> Result<(), TransportError> {
        Err(TransportError::Unsupported(
            "baud rate on a network connection",
        ))
    }
}

// ---- UDP ---------------------------------------------------------------

fn open_udp(
    path: &str,
    remote: Option<&str>,
    local: &str,
    read_timeout: Duration,
) -> Result<Connection, TransportError> {
    let remote = match remote {
        Some(r) => Some(resolve(r)?[0]),
        None => None,
    };
    let socket = UdpSocket::bind(resolve(local)?[0]).map_err(|e| bind_error(local, &e))?;
    socket.set_read_timeout(Some(read_timeout))?;
    let peer = Arc::new(Mutex::new(remote));
    Ok(Connection {
        info: info(TransportKind::Udp, path),
        reader: Box::new(UdpReader {
            socket: socket.try_clone()?,
            peer: Arc::clone(&peer),
            fixed: remote.is_some(),
            last: None,
            events: Vec::new(),
        }),
        control: Box::new(UdpControl { socket, peer }),
    })
}

struct UdpReader {
    socket: UdpSocket,
    /// Where replies go; follows the last sender unless the remote is fixed.
    peer: Arc<Mutex<Option<SocketAddr>>>,
    fixed: bool,
    last: Option<SocketAddr>,
    events: Vec<String>,
}

impl TransportReader for UdpReader {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError> {
        match self.socket.recv_from(buf) {
            Ok((n, from)) => {
                if self.last != Some(from) {
                    self.last = Some(from);
                    self.events.push(format!("datagrams from {from}"));
                    if !self.fixed {
                        *lock(&self.peer) = Some(from);
                    }
                }
                Ok(n)
            }
            // Windows reports an earlier send to a closed port this way.
            Err(e) if is_idle(e.kind()) || e.kind() == ErrorKind::ConnectionReset => Ok(0),
            Err(e) => Err(e.into()),
        }
    }

    fn take_event(&mut self) -> Option<String> {
        (!self.events.is_empty()).then(|| self.events.remove(0))
    }
}

struct UdpControl {
    socket: UdpSocket,
    peer: Arc<Mutex<Option<SocketAddr>>>,
}

impl TransportControl for UdpControl {
    fn write_all(&mut self, data: &[u8]) -> Result<(), TransportError> {
        let peer = (*lock(&self.peer)).ok_or_else(|| {
            TransportError::Io(std::io::Error::new(
                ErrorKind::NotConnected,
                "no remote address yet: nothing has been received",
            ))
        })?;
        let sent = self.socket.send_to(data, peer)?;
        if sent < data.len() {
            return Err(TransportError::Io(std::io::Error::new(
                ErrorKind::WriteZero,
                format!("only {sent} of {} bytes fit in one datagram", data.len()),
            )));
        }
        Ok(())
    }

    fn set_dtr(&mut self, _: bool) -> Result<(), TransportError> {
        Err(TransportError::Unsupported("DTR on a network connection"))
    }

    fn set_rts(&mut self, _: bool) -> Result<(), TransportError> {
        Err(TransportError::Unsupported("RTS on a network connection"))
    }

    fn line_states(&mut self) -> Result<LineStates, TransportError> {
        Err(TransportError::Unsupported(
            "modem lines on a network connection",
        ))
    }

    fn send_break(&mut self, _: Duration) -> Result<(), TransportError> {
        Err(TransportError::Unsupported("BREAK on a network connection"))
    }

    fn set_baud_rate(&mut self, _: u32) -> Result<(), TransportError> {
        Err(TransportError::Unsupported(
            "baud rate on a network connection",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    const T: Duration = Duration::from_millis(20);

    /// Reads until `want` bytes arrived (or 5 s pass).
    fn read_n(r: &mut dyn TransportReader, want: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf = [0u8; 256];
        let deadline = Instant::now() + Duration::from_secs(5);
        while out.len() < want {
            assert!(Instant::now() < deadline, "timed out after {out:?}");
            let n = r.read(&mut buf).unwrap();
            out.extend_from_slice(&buf[..n]);
        }
        out
    }

    fn events(r: &mut dyn TransportReader) -> Vec<String> {
        std::iter::from_fn(|| r.take_event()).collect()
    }

    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    /// A UDP port the OS hands out. A free TCP port may sit in a range
    /// Windows reserves for UDP (bind fails with WSAEACCES, os error 10013).
    fn free_udp_port() -> u16 {
        UdpSocket::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    #[test]
    fn parses_endpoints() {
        assert!(Endpoint::parse("COM3").is_none());
        assert!(Endpoint::parse("/dev/ttyUSB0").is_none());
        assert!(!is_network("COM3") && is_network("udp://x:1"));
        let ok = |p: &str| Endpoint::parse(p).unwrap().unwrap();
        assert_eq!(
            ok("tcp://192.168.1.10:8080"),
            Endpoint::TcpClient {
                remote: "192.168.1.10:8080".into()
            }
        );
        assert_eq!(
            ok("tcp-server://:9000"),
            Endpoint::TcpServer {
                local: "0.0.0.0:9000".into()
            }
        );
        assert_eq!(
            ok("udp://10.0.0.2:5000"),
            Endpoint::Udp {
                remote: Some("10.0.0.2:5000".into()),
                local: "0.0.0.0:0".into()
            }
        );
        assert_eq!(
            ok("udp://?local=:7000"),
            Endpoint::Udp {
                remote: None,
                local: "0.0.0.0:7000".into()
            }
        );
        assert_eq!(
            ok("udp://[::1]:5000?local=127.0.0.1:7000"),
            Endpoint::Udp {
                remote: Some("[::1]:5000".into()),
                local: "127.0.0.1:7000".into()
            }
        );
        for bad in [
            "tcp://host",
            "tcp://:80",
            "tcp://host:99999",
            "tcp-server://0.0.0.0",
            "udp://",
            "udp://?local=:0",
            "udp://h:1?local=x",
            "udp://h:1?mode=fast",
        ] {
            let err = Endpoint::parse(bad).unwrap().unwrap_err();
            assert!(
                matches!(err, TransportError::InvalidConfig(_)),
                "{bad}: {err}"
            );
        }
        assert!(matches!(
            open("COM3", T),
            Err(TransportError::InvalidConfig(_))
        ));
    }

    #[test]
    fn tcp_client_round_trip_and_disconnect() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let path = format!("tcp://{}", server.local_addr().unwrap());
        let mut c = open(&path, T).unwrap();
        assert_eq!(c.info.kind, TransportKind::TcpClient);
        assert_eq!(c.info.name, path);
        assert_eq!(c.info.capabilities, Capabilities::default());
        let (mut peer, _) = server.accept().unwrap();

        c.control.write_all(b"AT\r\n").unwrap();
        let mut got = [0u8; 4];
        peer.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"AT\r\n");
        let mut buf = [0u8; 16];
        assert_eq!(c.reader.read(&mut buf).unwrap(), 0, "idle read times out");
        peer.write_all(b"OK\r\n").unwrap();
        assert_eq!(read_n(c.reader.as_mut(), 4), b"OK\r\n");
        assert!(c.reader.take_event().is_none());

        drop(peer);
        assert!(matches!(
            c.reader.read(&mut buf),
            Err(TransportError::Disconnected)
        ));
        for (call, err) in [
            (c.control.set_dtr(true), "DTR"),
            (c.control.set_rts(true), "RTS"),
            (c.control.send_break(T), "BREAK"),
            (c.control.set_baud_rate(9600), "baud"),
        ] {
            assert!(call.unwrap_err().to_string().contains(err));
        }
        assert!(c.control.line_states().is_err());
    }

    #[test]
    fn tcp_client_reports_unreachable_servers() {
        let path = format!("tcp://127.0.0.1:{}", free_port());
        assert!(matches!(open(&path, T), Err(TransportError::NotFound(_))));
        assert!(matches!(
            open("tcp://no-such-host.invalid:80", T),
            Err(TransportError::NotFound(_))
        ));
    }

    #[test]
    fn tcp_server_serves_many_clients_at_once() {
        let port = free_port();
        let mut s = open(&format!("tcp-server://127.0.0.1:{port}"), T).unwrap();
        assert_eq!(s.info.kind, TransportKind::TcpServer);
        let mut buf = [0u8; 16];
        assert_eq!(s.reader.read(&mut buf).unwrap(), 0, "no client yet");
        assert!(
            s.control
                .write_all(b"x")
                .unwrap_err()
                .to_string()
                .contains("no client")
        );

        // One client alone: no "data from" markers.
        let mut a = TcpStream::connect(("127.0.0.1", port)).unwrap();
        a.write_all(b"hello").unwrap();
        assert_eq!(read_n(s.reader.as_mut(), 5), b"hello");
        let ev = events(s.reader.as_mut());
        assert_eq!(ev.len(), 1);
        assert!(ev[0].starts_with("client 127.0.0.1:") && ev[0].ends_with(" connected"));

        // A second client joins while the first stays.
        let mut b = TcpStream::connect(("127.0.0.1", port)).unwrap();
        b.write_all(b"from-b").unwrap();
        assert_eq!(read_n(s.reader.as_mut(), 6), b"from-b");
        let b_addr = b.local_addr().unwrap();
        let ev = events(s.reader.as_mut());
        assert!(ev[0].ends_with(" connected"), "{ev:?}");
        assert_eq!(ev[1], format!("data from {b_addr}"));
        a.write_all(b"from-a").unwrap();
        assert_eq!(read_n(s.reader.as_mut(), 6), b"from-a");
        assert_eq!(
            events(s.reader.as_mut()),
            [format!("data from {}", a.local_addr().unwrap())]
        );

        // Sending goes to both.
        s.control.write_all(b"hi").unwrap();
        let mut got = [0u8; 2];
        a.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        b.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        a.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"hi");
        b.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"hi");

        // One leaves; the other is still served.
        drop(a);
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut left = Vec::new();
        while left.is_empty() {
            assert!(Instant::now() < deadline, "the leave was not noticed");
            s.reader.read(&mut buf).unwrap();
            left = events(s.reader.as_mut());
        }
        assert!(left[0].ends_with(" disconnected"), "{left:?}");
        s.control.write_all(b"ok").unwrap();
        b.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"ok");

        // The port is taken while the session listens.
        assert!(matches!(
            open(&format!("tcp-server://127.0.0.1:{port}"), T),
            Err(TransportError::Busy(_))
        ));
    }

    #[test]
    fn tcp_server_refuses_clients_beyond_the_limit() {
        let port = free_port();
        let mut s = open(&format!("tcp-server://127.0.0.1:{port}"), T).unwrap();
        let clients: Vec<TcpStream> = (0..=MAX_SERVER_CLIENTS)
            .map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap())
            .collect();
        let mut buf = [0u8; 4];
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut seen = Vec::new();
        while seen.len() <= MAX_SERVER_CLIENTS {
            assert!(Instant::now() < deadline, "only {} events", seen.len());
            s.reader.read(&mut buf).unwrap();
            seen.extend(events(s.reader.as_mut()));
        }
        assert_eq!(
            seen.iter().filter(|e| e.ends_with(" connected")).count(),
            MAX_SERVER_CLIENTS
        );
        assert!(seen.iter().any(|e| e.contains("refused")), "{seen:?}");
        // The refused client is closed: it reads end of stream.
        let mut last = clients.last().unwrap();
        last.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        assert_eq!(last.read(&mut buf).unwrap_or(0), 0);
    }

    #[test]
    fn udp_sends_to_the_remote_and_receives_on_local() {
        let device = UdpSocket::bind("127.0.0.1:0").unwrap();
        device
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let local = free_udp_port();
        let path = format!(
            "udp://{}?local=127.0.0.1:{local}",
            device.local_addr().unwrap()
        );
        let mut u = open(&path, T).unwrap();
        assert_eq!(u.info.kind, TransportKind::Udp);

        u.control.write_all(b"ping").unwrap();
        let mut buf = [0u8; 16];
        let (n, from) = device.recv_from(&mut buf).unwrap();
        assert_eq!((&buf[..n], from.port()), (&b"ping"[..], local));

        device.send_to(b"pong", ("127.0.0.1", local)).unwrap();
        assert_eq!(read_n(u.reader.as_mut(), 4), b"pong");
        assert!(events(u.reader.as_mut())[0].starts_with("datagrams from 127.0.0.1:"));

        // A fixed remote stays the target even after other senders.
        let other = UdpSocket::bind("127.0.0.1:0").unwrap();
        other.send_to(b"!", ("127.0.0.1", local)).unwrap();
        assert_eq!(read_n(u.reader.as_mut(), 1), b"!");
        u.control.write_all(b"again").unwrap();
        let (n, _) = device.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"again");
        assert!(u.control.set_dtr(true).is_err());
    }

    #[test]
    fn udp_without_remote_answers_the_last_sender() {
        let local = free_udp_port();
        let mut u = open(&format!("udp://?local=127.0.0.1:{local}"), T).unwrap();
        assert!(
            u.control
                .write_all(b"x")
                .unwrap_err()
                .to_string()
                .contains("no remote")
        );

        let device = UdpSocket::bind("127.0.0.1:0").unwrap();
        device
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        device.send_to(b"hi", ("127.0.0.1", local)).unwrap();
        assert_eq!(read_n(u.reader.as_mut(), 2), b"hi");
        u.control.write_all(b"back").unwrap();
        let mut buf = [0u8; 16];
        let (n, _) = device.recv_from(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"back");

        let big = vec![0u8; 70_000];
        assert!(u.control.write_all(&big).is_err(), "larger than a datagram");
        assert!(matches!(
            open(&format!("udp://?local=127.0.0.1:{local}"), T),
            Err(TransportError::Busy(_))
        ));
    }
}
