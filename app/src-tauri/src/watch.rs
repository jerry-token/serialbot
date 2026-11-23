// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
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

use serialbot_core::SessionManager;

use crate::commands::{PortDto, PortLister, SessionDto, SignalsDto, session_dto, signals_dto};

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

/// Remembers the last seen session list, log ends and line levels.
#[derive(Debug, Default)]
pub struct ChangeDetector {
    sessions: Vec<SessionDto>,
    ends: BTreeMap<u32, (u64, &'static str)>,
    signals: BTreeMap<u32, SignalsDto>,
    ports: Option<Vec<PortDto>>,
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

    /// The port list, if it changed since the previous call (a device was
    /// plugged in or removed). The first call reports the list. Listing
    /// errors are ignored; the next call tries again.
    pub fn poll_ports(&mut self, lister: &PortLister) -> Option<Vec<PortDto>> {
        let ports: Vec<PortDto> = lister().ok()?.into_iter().map(Into::into).collect();
        if self.ports.as_ref() == Some(&ports) {
            return None;
        }
        self.ports = Some(ports.clone());
        Some(ports)
    }

    /// Line levels of open sessions that changed since the previous call
    /// (including sessions seen for the first time).
    ///
    /// Reading input lines can cost a USB round trip, so the caller runs
    /// this less often than [`ChangeDetector::poll`].
    pub fn poll_signals(&mut self, manager: &SessionManager) -> Vec<SignalsDto> {
        let mut seen = BTreeMap::new();
        let mut changed = Vec::new();
        for summary in manager.list() {
            if summary.state != serialbot_core::SessionState::Open {
                continue;
            }
            let Ok(session) = manager.get(summary.id) else {
                continue;
            };
            let dto = signals_dto(&session);
            if self.signals.get(&dto.id) != Some(&dto) {
                changed.push(dto.clone());
            }
            seen.insert(dto.id, dto);
        }
        self.signals = seen;
        changed
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use serialbot_core::transport::mock::{self, MockDevice};
    use serialbot_core::transport::serial::SerialConfig;
    use serialbot_core::{SessionId, SessionOptions, SystemClock};

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

    #[test]
    fn reports_port_list_changes() {
        use serialbot_core::transport::TransportError;
        use serialbot_core::transport::serial::{PortInfo, PortKind};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};
        let present = Arc::new(Mutex::new(vec!["COM3".to_owned()]));
        let failing = Arc::new(AtomicBool::new(false));
        let lister: Box<crate::commands::PortLister> = {
            let present = Arc::clone(&present);
            let failing = Arc::clone(&failing);
            Box::new(move || {
                if failing.load(Ordering::SeqCst) {
                    return Err(TransportError::PermissionDenied("udev".into()));
                }
                Ok(present
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|p| PortInfo {
                        path: p.clone(),
                        kind: PortKind::Unknown,
                    })
                    .collect())
            })
        };
        let mut detector = ChangeDetector::default();
        let first = detector.poll_ports(&lister).unwrap();
        assert_eq!(first[0].path, "COM3");
        assert_eq!(detector.poll_ports(&lister), None, "unchanged");
        present.lock().unwrap().push("COM4".into());
        assert_eq!(detector.poll_ports(&lister).unwrap().len(), 2, "plugged in");
        failing.store(true, Ordering::SeqCst);
        assert_eq!(detector.poll_ports(&lister), None, "errors are skipped");
        failing.store(false, Ordering::SeqCst);
        present.lock().unwrap().clear();
        assert_eq!(detector.poll_ports(&lister), Some(Vec::new()), "unplugged");
    }

    #[test]
    fn reports_signal_changes_of_open_sessions() {
        let (manager, device) = manager();
        let mut detector = ChangeDetector::default();
        assert!(detector.poll_signals(&manager).is_empty());
        let session = manager
            .open(&SerialConfig::new("mock0", 115_200), None)
            .unwrap();
        let first = detector.poll_signals(&manager);
        assert_eq!(first.len(), 1, "a new session is reported");
        assert_eq!(first[0].cts, Some(false));
        assert!(detector.poll_signals(&manager).is_empty());

        let dev = device.lock().unwrap().clone().unwrap();
        dev.set_line_states(serialbot_core::transport::LineStates {
            cts: true,
            ..Default::default()
        });
        assert_eq!(detector.poll_signals(&manager)[0].cts, Some(true));
        session.set_rts(true).unwrap();
        assert_eq!(detector.poll_signals(&manager)[0].rts, Some(true));

        manager.close(SessionId(1)).unwrap();
        assert!(
            detector.poll_signals(&manager).is_empty(),
            "closed sessions are skipped"
        );
    }
}
