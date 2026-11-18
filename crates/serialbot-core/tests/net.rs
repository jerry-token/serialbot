//! Network endpoints through the session manager, over real localhost
//! sockets: the same paths a user or an MCP client would open.
#![allow(clippy::unwrap_used)]

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use serialbot_core::transport::TransportKind;
use serialbot_core::transport::serial::SerialConfig;
use serialbot_core::{Cursor, EntryKind, Session, SessionManager, SessionState};

fn until(what: &str, check: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !check() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn markers(s: &Session) -> Vec<String> {
    s.read(Cursor(0), usize::MAX)
        .unwrap()
        .entries
        .into_iter()
        .filter_map(|e| match e.kind {
            EntryKind::Marker { label } => Some(label),
            EntryKind::Data { .. } => None,
        })
        .collect()
}

fn received(s: &Session) -> Vec<u8> {
    s.read(Cursor(0), usize::MAX)
        .unwrap()
        .entries
        .into_iter()
        .filter_map(|e| match e.kind {
            EntryKind::Data {
                direction: serialbot_core::Direction::Rx,
                bytes,
            } => Some(bytes.to_vec()),
            _ => None,
        })
        .flatten()
        .collect()
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn tcp_server_sessions_log_clients_and_data() {
    let manager = SessionManager::with_serial_ports();
    let port = free_port();
    let path = format!("tcp-server://127.0.0.1:{port}");
    let session = manager
        .open(&SerialConfig::new(&path, 115_200), None)
        .unwrap();
    assert_eq!(session.info().kind, TransportKind::TcpServer);

    let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
    client.write_all(b"temp=21\n").unwrap();
    until("data", || received(&session) == b"temp=21\n");
    session.write(b"ack\n").unwrap();
    let mut got = [0u8; 4];
    client.read_exact(&mut got).unwrap();
    assert_eq!(&got, b"ack\n");

    drop(client);
    until("leave marker", || markers(&session).len() == 2);
    let m = markers(&session);
    assert!(
        m[0].ends_with(" connected") && m[1].ends_with(" disconnected"),
        "{m:?}"
    );
    assert_eq!(session.state(), SessionState::Open, "keeps listening");

    // The same endpoint cannot be opened twice.
    assert!(
        manager
            .open(&SerialConfig::new(&path, 115_200), None)
            .is_err()
    );
}

#[test]
fn tcp_client_sessions_reconnect_when_the_server_returns() {
    let manager = SessionManager::with_serial_ports();
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = server.local_addr().unwrap();
    let path = format!("tcp://{addr}");
    let session = manager
        .open(&SerialConfig::new(&path, 115_200), None)
        .unwrap();
    let id = session.id();
    let (peer, _) = server.accept().unwrap();
    drop(peer);
    until("disconnect", || {
        session.state() == SessionState::Disconnected
    });

    // The server is still listening, so dialing again succeeds.
    manager.reconnect(id).unwrap();
    assert_eq!(session.state(), SessionState::Open);
    let (mut peer, _) = server.accept().unwrap();
    peer.write_all(b"back").unwrap();
    until("data after reconnect", || received(&session) == b"back");

    drop(server);
    drop(peer);
    until("second disconnect", || {
        session.state() == SessionState::Disconnected
    });
    assert!(manager.reconnect(id).is_err(), "nobody listens any more");
}
