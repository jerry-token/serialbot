//! Detects what changed in the session manager so the UI is told instead of
//! polling over IPC.
//!
//! A background thread calls [`ChangeDetector::poll`] about 60 times a
//! second. That is an in-memory comparison; IPC events are only sent when
//! something changed: the session list (opened, closed, state changes, also
//! by MCP clients) or the log of a session (new entries or a new state).
//! The UI then reads the new entries itself, so an event can never be lost
//! or arrive out of order relative to the data.

use std::collections::BTreeMap;

use serialx_core::SessionManager;

use crate::commands::{SessionDto, session_dto};

/// Changes since the previous poll.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Changes {
    /// The new session list, if it differs from the last one.
    pub sessions: Option<Vec<SessionDto>>,
    /// Sessions whose log or state changed, in id order.
    pub logs: Vec<u32>,
}

impl Changes {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.sessions.is_none() && self.logs.is_empty()
    }
}

/// Remembers the last seen session list and log ends.
#[derive(Debug, Default)]
pub struct ChangeDetector {
    sessions: Vec<SessionDto>,
    ends: BTreeMap<u32, (u64, &'static str)>,
}

impl ChangeDetector {
    /// Compares the manager with the previous call.
    pub fn poll(&mut self, manager: &SessionManager) -> Changes {
        let mut sessions = Vec::new();
        let mut ends = BTreeMap::new();
        for summary in manager.list() {
            let Ok(session) = manager.get(summary.id) else {
                continue; // Removed between the two calls.
            };
            let dto = session_dto(&summary);
            ends.insert(dto.id, (session.end().0, dto.state));
            sessions.push(dto);
        }
        let logs = ends
            .iter()
            .filter(|(id, now)| self.ends.get(id) != Some(now))
            .map(|(id, _)| *id)
            .collect();
        self.ends = ends;
        let sessions = (sessions != self.sessions).then(|| {
            self.sessions.clone_from(&sessions);
            sessions
        });
        Changes { sessions, logs }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use serialx_core::transport::mock::{self, MockDevice};
    use serialx_core::transport::serial::SerialConfig;
    use serialx_core::{SessionId, SessionOptions, SystemClock};

    use super::*;

    fn manager() -> (SessionManager, Arc<std::sync::Mutex<Option<MockDevice>>>) {
        let device = Arc::new(std::sync::Mutex::new(None));
        let slot = Arc::clone(&device);
        let manager = SessionManager::new(
            Box::new(move |config: &SerialConfig| {
                let (connection, dev) = mock::connection(&config.path, Duration::from_millis(5));
                *slot.lock().unwrap() = Some(dev);
                Ok(connection)
            }),
            Arc::new(SystemClock),
            SessionOptions::default(),
        );
        (manager, device)
    }

    /// Polls until `check` accepts the changes.
    fn poll_until(
        detector: &mut ChangeDetector,
        manager: &SessionManager,
        check: impl Fn(&Changes) -> bool,
    ) -> Changes {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let changes = detector.poll(manager);
            if check(&changes) {
                return changes;
            }
            assert!(Instant::now() < deadline, "change not detected");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn reports_sessions_logs_and_states_once() {
        let (manager, device) = manager();
        let mut detector = ChangeDetector::default();
        assert!(detector.poll(&manager).is_empty());

        manager
            .open(&SerialConfig::new("mock0", 115_200), None)
            .unwrap();
        let changes = detector.poll(&manager);
        let sessions = changes.sessions.unwrap();
        assert_eq!(sessions[0].port, "mock0");
        assert_eq!(changes.logs, [1], "a new session counts as changed");
        assert!(detector.poll(&manager).is_empty(), "nothing new");

        let dev = device.lock().unwrap().clone().unwrap();
        dev.send(b"hi");
        let changes = poll_until(&mut detector, &manager, |c| !c.logs.is_empty());
        assert_eq!(changes.logs, [1]);
        assert_eq!(changes.sessions, None, "the list did not change");

        dev.disconnect();
        let changes = poll_until(&mut detector, &manager, |c| c.sessions.is_some());
        assert_eq!(changes.sessions.unwrap()[0].state, "disconnected");
        assert_eq!(changes.logs, [1], "state changes notify the log too");

        manager.remove(SessionId(1)).unwrap();
        let changes = detector.poll(&manager);
        assert_eq!(changes.sessions, Some(Vec::new()));
        assert!(changes.logs.is_empty());
    }
}
