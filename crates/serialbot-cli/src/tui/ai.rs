//! The AI panel of `tui --mcp`: the MCP server's address and token, the
//! agent's actions as they are decided, and the one waiting for an answer
//! (`y` allow, `s` allow for this session, `n` or Esc deny).

use std::collections::VecDeque;
use std::sync::{Arc, mpsc};

use serialbot_mcp::approval::{ApprovalRequest, Approvals, Decision};

use crate::commands::mcp::{Event, Running};

/// Actions kept in the panel.
pub const KEEP: usize = 500;

/// One decided action, for the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Whether it went ahead.
    pub allowed: bool,
    /// `io.write mock0 "AT\r\n"`, and why it was refused.
    pub text: String,
}

/// The AI panel.
pub struct Ai {
    /// Where clients connect.
    pub url: String,
    /// The bearer token they send.
    pub token: String,
    /// Requests waiting for the user, oldest first.
    pub queue: Vec<ApprovalRequest>,
    /// Decided actions, oldest first.
    pub steps: VecDeque<Step>,
    approvals: Option<Arc<Approvals>>,
    events: mpsc::Receiver<Event>,
    running: Option<Running>,
}

impl std::fmt::Debug for Ai {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ai")
            .field("url", &self.url)
            .field("queue", &self.queue)
            .finish_non_exhaustive()
    }
}

impl Ai {
    /// The panel of a running server (stopped by [`Ai::stop`]).
    pub fn new(running: Running) -> Self {
        // The server's own events are read; this receiver stays empty.
        let mut ai = Self::from_parts(
            running.server.url(),
            running.token.clone(),
            running.approvals.clone(),
            mpsc::channel().1,
        );
        ai.running = Some(running);
        ai
    }

    /// A panel reading `events` and answering through `approvals`.
    pub fn from_parts(
        url: String,
        token: String,
        approvals: Option<Arc<Approvals>>,
        events: mpsc::Receiver<Event>,
    ) -> Self {
        Self {
            url,
            token,
            queue: Vec::new(),
            steps: VecDeque::new(),
            approvals,
            events,
            running: None,
        }
    }

    /// Takes in what the server reported.
    pub fn poll(&mut self) {
        let events = match &self.running {
            Some(running) => &running.events,
            None => &self.events,
        };
        let mut steps = Vec::new();
        while let Ok(event) = events.try_recv() {
            match event {
                Event::Queue(queue) => self.queue = queue,
                Event::Done(action, result) => {
                    let detail = if action.detail.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", action.detail)
                    };
                    let text = format!("{} {}{detail}", action.tool, action.port);
                    steps.push(match result {
                        Ok(()) => Step {
                            allowed: true,
                            text,
                        },
                        Err(why) => Step {
                            allowed: false,
                            text: format!("{text} — {why}"),
                        },
                    });
                }
                Event::Line(_) => {}
            }
        }
        for step in steps {
            if self.steps.len() == KEEP {
                self.steps.pop_front();
            }
            self.steps.push_back(step);
        }
    }

    /// The request waiting for an answer, if any.
    pub fn asking(&self) -> Option<&ApprovalRequest> {
        self.approvals.as_ref().and(self.queue.first())
    }

    /// Answers the waiting request; `false` if there is none.
    pub fn answer(&mut self, decision: Decision) -> bool {
        let (Some(approvals), Some(head)) = (&self.approvals, self.queue.first()) else {
            return false;
        };
        let _ = approvals.answer(head.id, decision);
        self.queue.remove(0);
        true
    }

    /// Stops the server (on quit).
    pub fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            running.stop();
        }
    }
}
