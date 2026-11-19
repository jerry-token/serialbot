//! Asking the user in the window before an AI agent changes anything.
//!
//! [`Approvals`] is the [`Guard`] of the embedded MCP server. Each action an
//! agent wants to carry out becomes a pending [`ApprovalRequest`]; the
//! window shows the queue and answers with a [`Decision`]. An unanswered
//! request is refused after the timeout, and one whose agent gave up (the
//! tool call was cancelled) leaves the queue by itself.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serialbot_core::SessionId;
use serialbot_mcp::guard::{Action, Approval, Guard};
use tokio::sync::oneshot;

/// How long a request waits for the user before it is refused.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// One action waiting for the user, as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    /// Identifier to answer with, unique for the app's lifetime.
    pub id: u64,
    /// Tool name, for example `io.write` or `command.call io.break`.
    pub tool: String,
    /// Session the action targets; `None` when opening a new one.
    pub session: Option<u32>,
    /// Port path or endpoint.
    pub port: String,
    /// What is sent or changed, in one line (may be empty).
    pub detail: String,
}

/// The user's answer to a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Decision {
    /// Allow this action only.
    Once,
    /// Allow this action and every later one on the same session, until
    /// the app restarts. For opening a port it means [`Decision::Once`].
    Session,
    /// Refuse it.
    Deny,
}

/// Called with the whole queue whenever it changes.
pub type Notify = Box<dyn Fn(Vec<ApprovalRequest>) + Send + Sync>;

#[derive(Default)]
struct State {
    next: u64,
    pending: BTreeMap<u64, (ApprovalRequest, oneshot::Sender<Decision>)>,
    trusted: HashSet<SessionId>,
}

/// Queue of agent actions waiting for the user.
pub struct Approvals {
    state: Mutex<State>,
    /// Whether to ask at all; when off every action goes ahead.
    enabled: AtomicBool,
    timeout: Duration,
    notify: Notify,
}

impl std::fmt::Debug for Approvals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Approvals")
            .field("enabled", &self.enabled)
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl Approvals {
    /// An enabled, empty queue that refuses unanswered requests after
    /// `timeout` and reports every change of the queue to `notify`.
    pub fn new(timeout: Duration, notify: Notify) -> Self {
        Self {
            state: Mutex::default(),
            enabled: AtomicBool::new(true),
            timeout,
            notify,
        }
    }

    /// Turns asking on or off. Turning it off does not answer requests
    /// already waiting.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    /// Whether actions are asked about.
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// The requests waiting, oldest first.
    pub fn pending(&self) -> Vec<ApprovalRequest> {
        Self::list(&self.lock())
    }

    /// Answers request `id`. Fails if it is no longer waiting (answered,
    /// timed out or abandoned by the agent).
    pub fn answer(&self, id: u64, decision: Decision) -> Result<(), String> {
        let mut state = self.lock();
        let (request, reply) = state
            .pending
            .remove(&id)
            .ok_or_else(|| format!("request {id} is no longer waiting"))?;
        if decision == Decision::Session
            && let Some(session) = request.session
        {
            state.trusted.insert(SessionId(session));
        }
        let list = Self::list(&state);
        drop(state);
        // The agent may have given up in the meantime; nothing to tell it.
        let _ = reply.send(decision);
        (self.notify)(list);
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn list(state: &State) -> Vec<ApprovalRequest> {
        state.pending.values().map(|(r, _)| r.clone()).collect()
    }

    /// Queues `action` unless it needs no asking; `None` means go ahead.
    fn enqueue(&self, action: Action) -> Option<(u64, oneshot::Receiver<Decision>)> {
        if !self.enabled() {
            return None;
        }
        let mut state = self.lock();
        if action.session.is_some_and(|s| state.trusted.contains(&s)) {
            return None;
        }
        state.next += 1;
        let id = state.next;
        let (tx, rx) = oneshot::channel();
        let request = ApprovalRequest {
            id,
            tool: action.tool,
            session: action.session.map(|s| s.0),
            port: action.port,
            detail: action.detail,
        };
        state.pending.insert(id, (request, tx));
        let list = Self::list(&state);
        drop(state);
        (self.notify)(list);
        Some((id, rx))
    }

    /// Drops request `id` if it is still waiting.
    fn withdraw(&self, id: u64) {
        let mut state = self.lock();
        if state.pending.remove(&id).is_some() {
            let list = Self::list(&state);
            drop(state);
            (self.notify)(list);
        }
    }
}

/// Withdraws a request when its future ends without an answer.
struct Withdraw<'a>(&'a Approvals, u64);

impl Drop for Withdraw<'_> {
    fn drop(&mut self) {
        self.0.withdraw(self.1);
    }
}

impl Guard for Approvals {
    fn approve(&self, action: Action) -> Approval<'_> {
        let tool = action.tool.clone();
        let queued = self.enqueue(action);
        Box::pin(async move {
            let Some((id, rx)) = queued else {
                return Ok(());
            };
            let _withdraw = Withdraw(self, id);
            match tokio::time::timeout(self.timeout, rx).await {
                Ok(Ok(Decision::Once | Decision::Session)) => Ok(()),
                Ok(Ok(Decision::Deny)) => Err(format!("the user declined {tool}")),
                Ok(Err(_)) => Err(format!("{tool} was not confirmed")),
                Err(_) => Err(format!(
                    "the user did not confirm {tool} within {} s",
                    self.timeout.as_secs()
                )),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn approvals(timeout: Duration) -> (Arc<Approvals>, Arc<Mutex<Vec<usize>>>) {
        let seen: Arc<Mutex<Vec<usize>>> = Arc::default();
        let log = Arc::clone(&seen);
        let notify: Notify = Box::new(move |list| log.lock().unwrap().push(list.len()));
        (Arc::new(Approvals::new(timeout, notify)), seen)
    }

    fn action(tool: &str, session: Option<u32>) -> Action {
        Action {
            tool: tool.into(),
            session: session.map(SessionId),
            port: "COM3".into(),
            detail: "\"AT\\r\\n\" (4 bytes)".into(),
        }
    }

    /// Starts approving `action` and waits until it is queued.
    async fn ask(
        a: &Arc<Approvals>,
        action: Action,
    ) -> tokio::task::JoinHandle<Result<(), String>> {
        let before = a.pending().len();
        let guard = Arc::clone(a);
        let task = tokio::spawn(async move { guard.approve(action).await });
        while a.pending().len() == before {
            tokio::task::yield_now().await;
        }
        task
    }

    #[tokio::test]
    async fn requests_wait_for_the_users_answer() {
        let (a, seen) = approvals(DEFAULT_TIMEOUT);
        let write = ask(&a, action("io.write", Some(1))).await;
        let open = ask(&a, action("session.open", None)).await;
        let pending = a.pending();
        assert_eq!(
            pending[0],
            ApprovalRequest {
                id: 1,
                tool: "io.write".into(),
                session: Some(1),
                port: "COM3".into(),
                detail: "\"AT\\r\\n\" (4 bytes)".into(),
            }
        );
        assert_eq!(pending[1].id, 2);
        a.answer(2, Decision::Deny).unwrap();
        assert_eq!(
            open.await.unwrap(),
            Err("the user declined session.open".into())
        );
        a.answer(1, Decision::Once).unwrap();
        assert_eq!(write.await.unwrap(), Ok(()));
        assert!(
            a.answer(1, Decision::Once)
                .unwrap_err()
                .contains("no longer waiting")
        );
        assert_eq!(*seen.lock().unwrap(), [1, 2, 1, 0]);

        // Once is once: the next write asks again.
        let again = ask(&a, action("io.write", Some(1))).await;
        a.answer(3, Decision::Once).unwrap();
        assert_eq!(again.await.unwrap(), Ok(()));
    }

    #[tokio::test]
    async fn a_session_can_be_trusted_until_restart() {
        let (a, _) = approvals(DEFAULT_TIMEOUT);
        let write = ask(&a, action("io.write", Some(1))).await;
        a.answer(1, Decision::Session).unwrap();
        assert_eq!(write.await.unwrap(), Ok(()));
        assert_eq!(a.approve(action("io.signals", Some(1))).await, Ok(()));
        assert!(a.pending().is_empty());
        // Other sessions, and opening ports, still ask.
        let other = ask(&a, action("io.write", Some(2))).await;
        let open = ask(&a, action("session.open", None)).await;
        a.answer(3, Decision::Session).unwrap();
        a.answer(2, Decision::Deny).unwrap();
        assert!(other.await.unwrap().is_err());
        assert_eq!(open.await.unwrap(), Ok(()));
        let open = ask(&a, action("session.open", None)).await;
        a.answer(4, Decision::Deny).unwrap();
        assert!(open.await.unwrap().is_err(), "opening is never trusted");
    }

    #[tokio::test]
    async fn disabled_approvals_let_everything_through() {
        let (a, seen) = approvals(DEFAULT_TIMEOUT);
        a.set_enabled(false);
        assert!(!a.enabled());
        assert_eq!(a.approve(action("io.write", Some(1))).await, Ok(()));
        assert!(seen.lock().unwrap().is_empty());
        a.set_enabled(true);
        assert!(a.enabled());
    }

    #[tokio::test]
    async fn unanswered_requests_are_refused_after_the_timeout() {
        let (a, seen) = approvals(Duration::from_millis(1100));
        let result = a.approve(action("io.write", Some(1))).await;
        assert_eq!(
            result,
            Err("the user did not confirm io.write within 1 s".into())
        );
        assert!(a.pending().is_empty());
        assert_eq!(*seen.lock().unwrap(), [1, 0]);
    }

    #[tokio::test]
    async fn abandoned_requests_leave_the_queue() {
        let (a, seen) = approvals(DEFAULT_TIMEOUT);
        let write = ask(&a, action("io.write", Some(1))).await;
        write.abort();
        assert!(write.await.unwrap_err().is_cancelled());
        assert!(a.pending().is_empty());
        assert_eq!(*seen.lock().unwrap(), [1, 0]);
        assert!(a.answer(1, Decision::Once).is_err());
        assert!(format!("{a:?}").contains("Approvals"));
    }

    #[test]
    fn decisions_come_from_the_window_in_camel_case() {
        let parsed: Vec<Decision> = serde_json::from_str(r#"["once", "session", "deny"]"#).unwrap();
        assert_eq!(parsed, [Decision::Once, Decision::Session, Decision::Deny]);
        let json = serde_json::to_string(&ApprovalRequest {
            id: 1,
            tool: "io.write".into(),
            session: None,
            port: "COM3".into(),
            detail: String::new(),
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"id":1,"tool":"io.write","session":null,"port":"COM3","detail":""}"#
        );
    }
}
