//! Transports: the only code that touches hardware or sockets.
//!
//! Opening a transport yields a [`Connection`], which is split into a
//! [`TransportReader`] (owned by a dedicated reader thread) and a
//! [`TransportControl`] (used for writes and line control). Splitting lets the
//! reader block on incoming data while other threads keep writing, without
//! sharing a lock on the hot path.
//!
//! Everything above this module is tested against [`mock`] connections, so
//! the session logic never needs real hardware in unit tests.

pub mod mock;
pub mod serial;

use std::time::Duration;

/// Errors a transport can report.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// The port or endpoint does not exist.
    #[error("port not found: {0}")]
    NotFound(String),
    /// The port exists but another program (or session) already holds it.
    #[error("port is busy, probably opened by another program: {0}")]
    Busy(String),
    /// The operating system refused access to the port.
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// The device went away (unplugged, peer closed).
    #[error("device disconnected")]
    Disconnected,
    /// The transport cannot do this (for example DTR on a TCP socket).
    #[error("not supported by this transport: {0}")]
    Unsupported(&'static str),
    /// The requested settings are invalid.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    /// Any other I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// What kind of link a connection is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportKind {
    /// A serial port (UART, USB CDC, Bluetooth SPP, PTY).
    Serial,
    /// An in-memory mock used by tests.
    Mock,
}

/// What a connection can do beyond reading and writing bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(clippy::struct_excessive_bools)] // Independent capability flags.
pub struct Capabilities {
    /// DTR/RTS can be driven.
    pub output_signals: bool,
    /// CTS/DSR/RI/CD can be read.
    pub input_signals: bool,
    /// A BREAK condition can be sent.
    pub send_break: bool,
    /// The baud rate can be changed while the port is open.
    pub live_baud_rate: bool,
}

/// Description of an open connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportInfo {
    /// Kind of link.
    pub kind: TransportKind,
    /// Human-readable name of the endpoint (for example `COM3`,
    /// `/dev/cu.usbserial-1410`).
    pub name: String,
    /// What the connection supports.
    pub capabilities: Capabilities,
}

/// State of the modem input lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(clippy::struct_excessive_bools)] // One flag per physical line.
pub struct LineStates {
    /// Clear To Send.
    pub cts: bool,
    /// Data Set Ready.
    pub dsr: bool,
    /// Ring Indicator.
    pub ri: bool,
    /// Carrier Detect.
    pub cd: bool,
}

/// Receiving half of a connection. Owned by exactly one reader thread.
pub trait TransportReader: Send {
    /// Reads available bytes into `buf`.
    ///
    /// Blocks for at most the connection's read timeout. Returns `Ok(0)`
    /// when no data arrived within the timeout, a positive count when data
    /// was read, and [`TransportError::Disconnected`] once the device is
    /// gone (after which every further call returns the same error).
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, TransportError>;
}

/// Sending and line-control half of a connection.
pub trait TransportControl: Send {
    /// Writes all of `data`, blocking until it was handed to the OS.
    fn write_all(&mut self, data: &[u8]) -> Result<(), TransportError>;

    /// Drives the DTR line.
    fn set_dtr(&mut self, level: bool) -> Result<(), TransportError>;

    /// Drives the RTS line.
    fn set_rts(&mut self, level: bool) -> Result<(), TransportError>;

    /// Reads the modem input lines.
    fn line_states(&mut self) -> Result<LineStates, TransportError>;

    /// Holds a BREAK condition for `duration`.
    fn send_break(&mut self, duration: Duration) -> Result<(), TransportError>;

    /// Changes the baud rate without closing the port.
    fn set_baud_rate(&mut self, baud_rate: u32) -> Result<(), TransportError>;
}

/// An open link, ready to be split between a reader thread and a writer.
pub struct Connection {
    /// Description of the link.
    pub info: TransportInfo,
    /// Receiving half.
    pub reader: Box<dyn TransportReader>,
    /// Sending and control half.
    pub control: Box<dyn TransportControl>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}
