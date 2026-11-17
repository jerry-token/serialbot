//! Core engine of serialx.
//!
//! This crate holds everything that does not depend on a user interface:
//! the timestamped log buffer, transports (serial, TCP, UDP, ...) and the
//! session manager. The desktop app, the MCP server and the command-line
//! mode all talk to the device only through this crate.

pub mod log;

pub use log::{
    Cursor, Direction, ENTRY_OVERHEAD, Entry, EntryKind, LogBuffer, LogError, ReadResult,
};
