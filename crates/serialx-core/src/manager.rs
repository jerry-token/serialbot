//! The session manager: the single owner of every open port.
//!
//! Windows, the MCP server and the CLI all go through one manager per
//! process, so a port can never be opened twice by the same app and every
//! client sees the same sessions.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::clock::Clock;
use crate::session::{Session, SessionError, SessionId, SessionOptions, SessionState};
use crate::transport::serial::SerialConfig;
use crate::transport::{Connection, TransportError, TransportInfo};

/// Opens a connection for a configuration. Production code uses
/// [`crate::transport::serial::open`]; tests inject mock connections.
pub type Opener = dyn Fn(&SerialConfig) -> Result<Connection, TransportError> + Send + Sync;

/// Errors returned by the manager.
#[derive(Debug, thiserror::Error)]
pub enum ManagerError {
    /// No session has this id.
    #[error("no session {0}")]
    UnknownSession(SessionId),
    /// No session has this alias.
    #[error("no session named {0:?}")]
    UnknownAlias(String),
    /// Another call is opening this port right now.
    #[error("{0} is being opened by another request")]
    PortOpening(String),
    /// Another call that is opening a port right now reserved this alias.
    #[error("the name {0:?} is reserved by a port being opened")]
    AliasReserved(String),
    /// Another live session already holds the port.
    #[error("{port} is already open in session {session}")]
    PortInUse {
        /// The port that was requested.
        port: String,
        /// The session holding it.
        session: SessionId,
    },
    /// The alias is taken by another session.
    #[error("the name {alias:?} is already used by session {session}")]
    AliasInUse {
        /// The requested alias.
        alias: String,
        /// The session using it.
        session: SessionId,
    },
    /// Opening or running the session failed.
    #[error(transparent)]
    Session(#[from] SessionError),
}

impl From<TransportError> for ManagerError {
    fn from(e: TransportError) -> Self {
        Self::Session(SessionError::Transport(e))
    }
}

/// Snapshot of a session for listings (UI tabs, `session.list`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    /// Session id.
    pub id: SessionId,
    /// User-chosen name, if any.
    pub alias: Option<String>,
    /// Connection description.
    pub info: TransportInfo,
    /// Lifecycle state.
    pub state: SessionState,
}

/// Owns all sessions of the process.
pub struct SessionManager {
    sessions: Mutex<Registry>,
    next_id: Mutex<u32>,
    opener: Box<Opener>,
    clock: Arc<dyn Clock>,
    options: SessionOptions,
}

impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager")
            .field("sessions", &self.list())
            .finish_non_exhaustive()
    }
}

/// Sessions plus the ports currently being opened.
#[derive(Default)]
struct Registry {
    sessions: BTreeMap<SessionId, Arc<Session>>,
    /// Ports reserved by an open call in progress (normalized names).
    opening: HashSet<String>,
    /// Aliases reserved by an open call in progress.
    opening_aliases: HashSet<String>,
}

impl SessionManager {
    /// Creates a manager that opens ports with `opener`.
    pub fn new(opener: Box<Opener>, clock: Arc<dyn Clock>, options: SessionOptions) -> Self {
        Self {
            sessions: Mutex::new(Registry::default()),
            next_id: Mutex::new(1),
            opener,
            clock,
            options,
        }
    }

    /// Creates a manager for real serial ports using the system clock.
    pub fn with_serial_ports() -> Self {
        Self::new(
            Box::new(crate::transport::serial::open),
            Arc::new(crate::clock::SystemClock),
            SessionOptions::default(),
        )
    }

    /// Opens `config.path` in a new session.
    ///
    /// Fails with [`ManagerError::PortInUse`] if an open session already
    /// holds the port (closed or disconnected sessions do not block it), and
    /// with [`ManagerError::AliasInUse`] if `alias` is taken.
    pub fn open(
        &self,
        config: &SerialConfig,
        alias: Option<String>,
    ) -> Result<Arc<Session>, ManagerError> {
        let wanted = port_key(&config.path);
        // Reserve the port under the lock, then open it *without* the lock:
        // an OS open can take seconds (Bluetooth ports on Windows), and
        // listing sessions must not stall meanwhile.
        {
            let mut reg = self.lock();
            if let Some(holder) = reg
                .sessions
                .values()
                .find(|s| s.state() == SessionState::Open && port_key(&s.info().name) == wanted)
            {
                return Err(ManagerError::PortInUse {
                    port: config.path.clone(),
                    session: holder.id(),
                });
            }
            if reg.opening.contains(&wanted) {
                return Err(ManagerError::PortOpening(config.path.clone()));
            }
            if let Some(name) = &alias {
                check_alias_free(&reg.sessions, name, None)?;
                if reg.opening_aliases.contains(name) {
                    return Err(ManagerError::AliasReserved(name.clone()));
                }
                reg.opening_aliases.insert(name.clone());
            }
            reg.opening.insert(wanted.clone());
        }
        let started = (self.opener)(config)
            .map_err(ManagerError::from)
            .and_then(|connection| {
                Session::start(
                    self.allocate_id(),
                    connection,
                    Arc::clone(&self.clock),
                    self.options,
                )
                .map_err(ManagerError::from)
            });
        let mut reg = self.lock();
        reg.opening.remove(&wanted);
        if let Some(name) = &alias {
            reg.opening_aliases.remove(name);
        }
        let session = Arc::new(started?);
        session.set_alias(alias);
        reg.sessions.insert(session.id(), Arc::clone(&session));
        Ok(session)
    }

    /// Lists every session, open or not, in id order.
    pub fn list(&self) -> Vec<SessionSummary> {
        self.lock()
            .sessions
            .values()
            .map(|s| SessionSummary {
                id: s.id(),
                alias: s.alias(),
                info: s.info().clone(),
                state: s.state(),
            })
            .collect()
    }

    /// Looks a session up by id.
    pub fn get(&self, id: SessionId) -> Result<Arc<Session>, ManagerError> {
        self.lock()
            .sessions
            .get(&id)
            .cloned()
            .ok_or(ManagerError::UnknownSession(id))
    }

    /// Looks a session up by alias (exact match).
    pub fn find_by_alias(&self, alias: &str) -> Result<Arc<Session>, ManagerError> {
        self.lock()
            .sessions
            .values()
            .find(|s| s.alias().as_deref() == Some(alias))
            .cloned()
            .ok_or_else(|| ManagerError::UnknownAlias(alias.to_owned()))
    }

    /// Renames a session; `None` clears the alias.
    pub fn set_alias(&self, id: SessionId, alias: Option<String>) -> Result<(), ManagerError> {
        let reg = self.lock();
        let session = reg
            .sessions
            .get(&id)
            .ok_or(ManagerError::UnknownSession(id))?;
        if let Some(name) = &alias {
            check_alias_free(&reg.sessions, name, Some(id))?;
        }
        session.set_alias(alias);
        Ok(())
    }

    /// Closes the port of a session but keeps it listed with its log.
    pub fn close(&self, id: SessionId) -> Result<(), ManagerError> {
        let session = self.get(id)?;
        session.close();
        Ok(())
    }

    /// Closes a session and forgets it, dropping its log.
    pub fn remove(&self, id: SessionId) -> Result<(), ManagerError> {
        let session = self
            .lock()
            .sessions
            .remove(&id)
            .ok_or(ManagerError::UnknownSession(id))?;
        session.close();
        Ok(())
    }

    /// Closes every session (application shutdown).
    pub fn close_all(&self) {
        let sessions: Vec<_> = self.lock().sessions.values().cloned().collect();
        for s in sessions {
            s.close();
        }
    }

    fn allocate_id(&self) -> SessionId {
        let mut next = self.next_id.lock().unwrap_or_else(PoisonError::into_inner);
        let id = SessionId(*next);
        *next = next.wrapping_add(1).max(1);
        id
    }

    fn lock(&self) -> MutexGuard<'_, Registry> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn check_alias_free(
    sessions: &BTreeMap<SessionId, Arc<Session>>,
    alias: &str,
    except: Option<SessionId>,
) -> Result<(), ManagerError> {
    match sessions
        .values()
        .find(|s| Some(s.id()) != except && s.alias().as_deref() == Some(alias))
    {
        Some(s) => Err(ManagerError::AliasInUse {
            alias: alias.to_owned(),
            session: s.id(),
        }),
        None => Ok(()),
    }
}

/// Normalizes a port name for comparison: Windows names are
/// case-insensitive and may carry the `\\.\` prefix.
fn port_key(path: &str) -> String {
    if cfg!(windows) {
        path.strip_prefix(r"\\.\")
            .unwrap_or(path)
            .to_ascii_uppercase()
    } else {
        path.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::ManualClock;
    use crate::transport::mock::{self, MockDevice};
    use std::time::Duration;

    /// A manager whose opener creates mock connections and hands the device
    /// ends to the test.
    fn manager() -> (SessionManager, Arc<Mutex<Vec<MockDevice>>>) {
        let devices = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&devices);
        let opener = move |c: &SerialConfig| {
            if c.path == "missing" {
                return Err(TransportError::NotFound(c.path.clone()));
            }
            let (conn, dev) = mock::connection(&c.path, Duration::from_millis(5));
            sink.lock().unwrap().push(dev);
            Ok(conn)
        };
        let m = SessionManager::new(
            Box::new(opener),
            Arc::new(ManualClock::new(0)),
            SessionOptions::default(),
        );
        (m, devices)
    }

    fn cfg(path: &str) -> SerialConfig {
        SerialConfig::new(path, 115_200)
    }

    #[test]
    fn open_assigns_increasing_ids_and_lists_sessions() {
        let (m, _d) = manager();
        let a = m.open(&cfg("COM3"), Some("main board".into())).unwrap();
        let b = m.open(&cfg("COM5"), None).unwrap();
        assert_eq!((a.id(), b.id()), (SessionId(1), SessionId(2)));
        let list = m.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].alias.as_deref(), Some("main board"));
        assert_eq!(list[0].info.name, "COM3");
        assert_eq!(list[1].state, SessionState::Open);
        assert!(format!("{m:?}").contains("COM5"));
    }

    #[test]
    fn the_same_port_cannot_be_opened_twice() {
        let (m, _d) = manager();
        let a = m.open(&cfg("COM3"), None).unwrap();
        let err = m.open(&cfg("COM3"), None).unwrap_err();
        assert!(
            matches!(err, ManagerError::PortInUse { ref port, session } if port == "COM3" && session == a.id())
        );
        // Once closed, the port can be opened again in a new session.
        m.close(a.id()).unwrap();
        let b = m.open(&cfg("COM3"), None).unwrap();
        assert_ne!(a.id(), b.id());
    }

    #[test]
    fn port_names_are_compared_like_the_os_does() {
        let (m, _d) = manager();
        m.open(&cfg("COM3"), None).unwrap();
        let again = m.open(&cfg("com3"), None);
        if cfg!(windows) {
            assert!(matches!(again, Err(ManagerError::PortInUse { .. })));
        } else {
            assert!(again.is_ok(), "names are case-sensitive on Unix");
        }
    }

    #[test]
    fn a_disconnected_session_frees_its_port() {
        let (m, devices) = manager();
        let a = m.open(&cfg("COM3"), None).unwrap();
        devices.lock().unwrap()[0].disconnect();
        while a.state() == SessionState::Open {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(m.open(&cfg("COM3"), None).is_ok());
    }

    #[test]
    fn open_errors_are_passed_through() {
        let (m, _d) = manager();
        let err = m.open(&cfg("missing"), None).unwrap_err();
        assert!(matches!(
            err,
            ManagerError::Session(SessionError::Transport(TransportError::NotFound(_)))
        ));
        assert!(m.list().is_empty());
    }

    #[test]
    fn aliases_are_unique_and_searchable() {
        let (m, _d) = manager();
        let a = m.open(&cfg("COM3"), Some("modem".into())).unwrap();
        let b = m.open(&cfg("COM5"), None).unwrap();
        assert!(matches!(
            m.open(&cfg("COM7"), Some("modem".into())),
            Err(ManagerError::AliasInUse { session, .. }) if session == a.id()
        ));
        assert!(matches!(
            m.set_alias(b.id(), Some("modem".into())),
            Err(ManagerError::AliasInUse { .. })
        ));
        // Renaming a session to its own alias is fine.
        m.set_alias(a.id(), Some("modem".into())).unwrap();
        assert_eq!(m.find_by_alias("modem").unwrap().id(), a.id());
        m.set_alias(a.id(), None).unwrap();
        assert!(matches!(
            m.find_by_alias("modem"),
            Err(ManagerError::UnknownAlias(_))
        ));
    }

    #[test]
    fn unknown_ids_are_reported() {
        let (m, _d) = manager();
        let id = SessionId(42);
        assert!(matches!(m.get(id), Err(ManagerError::UnknownSession(_))));
        assert!(matches!(m.close(id), Err(ManagerError::UnknownSession(_))));
        assert!(matches!(m.remove(id), Err(ManagerError::UnknownSession(_))));
        assert!(matches!(
            m.set_alias(id, None),
            Err(ManagerError::UnknownSession(_))
        ));
    }

    #[test]
    fn close_keeps_the_session_listed_and_remove_forgets_it() {
        let (m, _d) = manager();
        let a = m.open(&cfg("COM3"), None).unwrap();
        m.close(a.id()).unwrap();
        assert_eq!(m.list()[0].state, SessionState::Closed);
        m.remove(a.id()).unwrap();
        assert!(m.list().is_empty());
    }

    #[test]
    fn close_all_closes_every_session() {
        let (m, _d) = manager();
        m.open(&cfg("COM3"), None).unwrap();
        m.open(&cfg("COM5"), None).unwrap();
        m.close_all();
        assert!(m.list().iter().all(|s| s.state == SessionState::Closed));
    }

    #[test]
    fn concurrent_opens_of_one_port_yield_exactly_one_session() {
        let (m, _d) = manager();
        let m = Arc::new(m);
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let m = Arc::clone(&m);
                std::thread::spawn(move || m.open(&cfg("COM3"), None).is_ok())
            })
            .collect();
        let opened = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|ok| *ok)
            .count();
        assert_eq!(opened, 1);
    }

    #[test]
    fn listing_does_not_wait_for_a_slow_open() {
        use std::sync::mpsc;
        // The opener reports that it was entered, then blocks until released.
        let (entered_tx, entered_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let entered_tx = Mutex::new(entered_tx);
        let release_rx = Mutex::new(release_rx);
        let opener = move |c: &SerialConfig| {
            entered_tx.lock().unwrap().send(()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            Ok(mock::connection(&c.path, Duration::from_millis(5)).0)
        };
        let m = Arc::new(SessionManager::new(
            Box::new(opener),
            Arc::new(ManualClock::new(0)),
            SessionOptions::default(),
        ));
        let opening = {
            let m = Arc::clone(&m);
            std::thread::spawn(move || m.open(&cfg("COM9"), Some("bt".into())).map(|s| s.id()))
        };
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();

        // The first open is now blocked inside the opener with its
        // reservations held. None of these calls may block.
        assert!(m.list().is_empty());
        assert!(matches!(
            m.open(&cfg("COM9"), None),
            Err(ManagerError::PortOpening(ref p)) if p == "COM9"
        ));
        assert!(matches!(
            m.open(&cfg("COM1"), Some("bt".into())),
            Err(ManagerError::AliasReserved(ref a)) if a == "bt"
        ));

        release_tx.send(()).unwrap();
        let id = opening.join().unwrap().unwrap();
        assert_eq!(m.get(id).unwrap().alias().as_deref(), Some("bt"));
    }

    #[test]
    fn a_failed_open_releases_its_reservations() {
        let (m, _d) = manager();
        for _ in 0..2 {
            // A leaked reservation would turn the second attempt into
            // PortOpening / AliasReserved.
            assert!(matches!(
                m.open(&cfg("missing"), Some("x".into())),
                Err(ManagerError::Session(_))
            ));
        }
        assert!(m.open(&cfg("COM3"), Some("x".into())).is_ok());
    }

    #[test]
    fn real_serial_manager_reports_missing_ports() {
        let m = SessionManager::with_serial_ports();
        let path = if cfg!(windows) {
            "COM250"
        } else {
            "/dev/serialx-does-not-exist"
        };
        assert!(matches!(
            m.open(&cfg(path), None),
            Err(ManagerError::Session(SessionError::Transport(
                TransportError::NotFound(_)
            )))
        ));
    }

    #[test]
    fn transport_errors_convert_into_manager_errors() {
        let e: ManagerError = TransportError::Disconnected.into();
        assert!(e.to_string().contains("disconnected"));
    }
}
