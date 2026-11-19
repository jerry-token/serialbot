//! Core engine of serialbot.
//!
//! This crate holds everything that does not depend on a user interface:
//! the timestamped log buffer, transports (serial, TCP, UDP, ...) and the
//! session manager. The desktop app, the MCP server and the command-line
//! mode all talk to the device only through this crate.

pub mod clock;
pub mod encoding;
pub mod log;
pub mod manager;
pub mod session;
pub mod transport;

pub use clock::{Clock, ManualClock, SystemClock};
pub use log::{
    Actor, Cursor, Direction, ENTRY_OVERHEAD, Entry, EntryKind, LogBuffer, LogError, ReadResult,
};
pub use manager::{ManagerError, SessionManager, SessionSummary};
pub use session::{
    OutputSignals, Session, SessionError, SessionId, SessionOptions, SessionState, WaitMatch,
};
