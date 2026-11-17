//! The curated MCP tool set.
//!
//! Tools address a session by its id (`"s1"`) or its alias. Reads and waits
//! work with log cursors: every entry has a sequence number, a cursor is
//! the number of the next entry to look at, and each result returns the
//! cursor to continue from, so an agent never sees an entry twice or misses
//! one without being told (`missed`).

use std::sync::Arc;
use std::time::Duration;

use regex::bytes::Regex;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serialx_core::transport::serial::{
    DataBits, FlowControl, Parity, PortInfo, PortKind, SerialConfig, StopBits,
};
use serialx_core::transport::{TransportError, TransportKind};
use serialx_core::{
    Cursor, Direction, EntryKind, Session, SessionId, SessionManager, SessionState, SessionSummary,
    WaitMatch,
};

use crate::payload::{self, Format};

/// Lists the ports present on the machine; replaceable in tests.
pub type PortLister = dyn Fn() -> Result<Vec<PortInfo>, TransportError> + Send + Sync;

/// Default payload budget of one `io.read`.
pub const DEFAULT_READ_BYTES: usize = 16 * 1024;
/// Largest payload budget `io.read` accepts.
pub const MAX_READ_BYTES: usize = 1024 * 1024;
/// Default timeout of `io.wait_for` and `io.send_and_expect`.
pub const DEFAULT_TIMEOUT_MS: u64 = 5_000;
/// Longest timeout a single wait accepts.
pub const MAX_TIMEOUT_MS: u64 = 60_000;
/// Default number of trailing received bytes a pattern may span.
pub const DEFAULT_MATCH_WINDOW: usize = 64 * 1024;
/// Largest regex the wait tools compile, in bytes of compiled program.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

const INSTRUCTIONS: &str = "Drives serial ports opened in serialx. \
Use port.list to find ports, session.open to open one (or session.list to \
reuse an open session), then io.write / io.read / io.wait_for / \
io.send_and_expect, and io.signals for DTR/RTS. Every log entry has a sequence number; pass the \
`cursor` a result returns to the next call so nothing is read twice. \
io.write returns the cursor taken just before sending: pass it to \
io.wait_for to catch replies that arrive very fast.";

/// MCP handler exposing serialx sessions as tools.
#[derive(Clone)]
pub struct SerialTools {
    manager: Arc<SessionManager>,
    list_ports: Arc<PortLister>,
    tool_router: ToolRouter<Self>,
}

impl std::fmt::Debug for SerialTools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SerialTools")
            .field("manager", &self.manager)
            .finish_non_exhaustive()
    }
}

// ---- arguments -----------------------------------------------------------

/// Parity argument.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ParityArg {
    /// No parity bit.
    #[default]
    None,
    /// Odd parity.
    Odd,
    /// Even parity.
    Even,
}

/// Flow-control argument.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum FlowControlArg {
    /// No flow control.
    #[default]
    None,
    /// XON/XOFF.
    Software,
    /// RTS/CTS.
    Hardware,
}

/// Arguments of `session.open`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct OpenArgs {
    /// OS name of the port as listed by port.list, e.g. "COM3" or
    /// "/dev/ttyUSB0".
    pub port: String,
    /// Baud rate (default 115200).
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    /// Data bits, 5 to 8 (default 8).
    #[serde(default = "default_data_bits")]
    pub data_bits: u8,
    /// Parity (default none).
    #[serde(default)]
    pub parity: ParityArg,
    /// Stop bits, 1 or 2 (default 1).
    #[serde(default = "default_stop_bits")]
    pub stop_bits: u8,
    /// Flow control (default none).
    #[serde(default)]
    pub flow_control: FlowControlArg,
    /// Optional unique name for the session, e.g. "esp32".
    #[serde(default)]
    pub alias: Option<String>,
}

fn default_baud_rate() -> u32 {
    115_200
}
fn default_data_bits() -> u8 {
    8
}
fn default_stop_bits() -> u8 {
    1
}

/// Arguments of `io.write`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct WriteArgs {
    /// Session id or alias.
    pub session: String,
    /// Data to send as UTF-8 text. Include line endings such as "\r\n"
    /// yourself. Give either `text` or `hex`.
    #[serde(default)]
    pub text: Option<String>,
    /// Data to send as hex digits, e.g. "01 03 00 00". Give either `text`
    /// or `hex`.
    #[serde(default)]
    pub hex: Option<String>,
}

/// Arguments of `io.read`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ReadArgs {
    /// Session id or alias.
    pub session: String,
    /// Cursor to read from (default 0: the oldest entry still kept).
    #[serde(default)]
    pub cursor: u64,
    /// Payload budget in bytes (default 16384, max 1048576). At least one
    /// entry is returned even if it is larger.
    #[serde(default)]
    pub max_bytes: Option<usize>,
    /// How data is rendered (default text).
    #[serde(default)]
    pub format: Format,
}

/// Arguments of `io.wait_for`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct WaitArgs {
    /// Session id or alias.
    pub session: String,
    /// Regular expression (Rust regex syntax) matched against received
    /// bytes. Use (?s) to let `.` match newlines.
    pub pattern: String,
    /// Search received data from this cursor (default: only data that
    /// arrives after the call starts). Pass the cursor io.write returned
    /// so fast replies are not missed.
    #[serde(default)]
    pub cursor: Option<u64>,
    /// Timeout in milliseconds (default 5000, max 60000).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// How the match is rendered (default text).
    #[serde(default)]
    pub format: Format,
}

/// Arguments of `io.signals`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SignalsArgs {
    /// Session id or alias.
    pub session: String,
    /// New DTR level; omit to leave it unchanged.
    #[serde(default)]
    pub dtr: Option<bool>,
    /// New RTS level; omit to leave it unchanged.
    #[serde(default)]
    pub rts: Option<bool>,
}

/// Arguments of `io.send_and_expect`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SendExpectArgs {
    /// Session id or alias.
    pub session: String,
    /// Data to send as UTF-8 text. Give either `text` or `hex`.
    #[serde(default)]
    pub text: Option<String>,
    /// Data to send as hex digits. Give either `text` or `hex`.
    #[serde(default)]
    pub hex: Option<String>,
    /// Regular expression matched against what is received after sending.
    pub pattern: String,
    /// Timeout in milliseconds (default 5000, max 60000).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// How the match is rendered (default text).
    #[serde(default)]
    pub format: Format,
}

// ---- results -------------------------------------------------------------

/// A session in tool results.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct SessionOut {
    /// Session id, e.g. "s1".
    pub id: String,
    /// Alias, if set.
    pub alias: Option<String>,
    /// Port or device name.
    pub port: String,
    /// "serial" or "mock".
    pub transport: String,
    /// "open", "disconnected", "closed" or "failed".
    pub state: String,
    /// Reason, when the state is "failed".
    pub error: Option<String>,
}

impl From<SessionSummary> for SessionOut {
    fn from(s: SessionSummary) -> Self {
        let (state, error) = match s.state {
            SessionState::Open => ("open", None),
            SessionState::Disconnected => ("disconnected", None),
            SessionState::Closed => ("closed", None),
            SessionState::Failed(reason) => ("failed", Some(reason)),
        };
        Self {
            id: s.id.to_string(),
            alias: s.alias,
            port: s.info.name,
            transport: match s.info.kind {
                TransportKind::Serial => "serial",
                TransportKind::Mock => "mock",
            }
            .into(),
            state: state.into(),
            error,
        }
    }
}

/// Result of `session.list`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct SessionsOut {
    /// All sessions, oldest first.
    pub sessions: Vec<SessionOut>,
}

/// A port in `port.list`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct PortOut {
    /// Name to pass to session.open.
    pub port: String,
    /// "usb", "bluetooth", "pci" or "unknown".
    pub kind: String,
    /// USB vendor id as 4 hex digits.
    pub vid: Option<String>,
    /// USB product id as 4 hex digits.
    pub pid: Option<String>,
    /// USB serial number.
    pub serial_number: Option<String>,
    /// USB manufacturer string.
    pub manufacturer: Option<String>,
    /// USB product string.
    pub product: Option<String>,
    /// Id of the open session using this port, if any.
    pub session: Option<String>,
}

/// Result of `port.list`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct PortsOut {
    /// Ports present on the machine.
    pub ports: Vec<PortOut>,
}

/// Result of `io.write`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct WriteOut {
    /// Number of bytes sent.
    pub bytes: usize,
    /// Sequence number of the logged TX entry.
    pub seq: u64,
    /// Cursor taken just before sending; pass it to `io.wait_for`.
    pub cursor: u64,
}

/// One log entry in `io.read`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct EntryOut {
    /// Sequence number.
    pub seq: u64,
    /// Microseconds since the Unix epoch.
    pub timestamp_us: u64,
    /// "rx", "tx" or "marker".
    pub kind: String,
    /// Data in the requested format, or the marker label.
    pub data: String,
}

/// Result of `io.read`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct ReadOut {
    /// Entries in order.
    pub entries: Vec<EntryOut>,
    /// Cursor for the next read.
    pub cursor: u64,
    /// Entries between the requested cursor and the oldest kept entry that
    /// were already dropped from the log.
    pub missed: u64,
    /// Whether more entries are already available after `cursor`.
    pub more: bool,
}

/// Result of `io.signals`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct SignalsOut {
    /// DTR level last set in this session (null: never set).
    pub dtr: Option<bool>,
    /// RTS level last set in this session (null: never set).
    pub rts: Option<bool>,
    /// Clear To Send (null: cannot be read on this port).
    pub cts: Option<bool>,
    /// Data Set Ready.
    pub dsr: Option<bool>,
    /// Ring Indicator.
    pub ri: Option<bool>,
    /// Carrier Detect.
    pub cd: Option<bool>,
    /// Why the input lines could not be read, if they could not.
    pub input_error: Option<String>,
}

/// Result of `io.wait_for` and `io.send_and_expect`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct WaitOut {
    /// Whether the pattern matched before the timeout.
    pub matched: bool,
    /// The matched bytes in the requested format.
    pub data: Option<String>,
    /// Sequence number of the entry where the match ends.
    pub seq: Option<u64>,
    /// After a match, the cursor just past it; after a timeout, the cursor
    /// the search started from, so io.read can show what did arrive.
    pub cursor: u64,
}

impl WaitOut {
    fn from_result(found: Option<WaitMatch>, from: Cursor, format: Format) -> Self {
        match found {
            Some(m) => Self {
                matched: true,
                data: Some(payload::render(&m.matched, format)),
                seq: Some(m.seq),
                cursor: m.next.0,
            },
            None => Self {
                matched: false,
                data: None,
                seq: None,
                cursor: from.0,
            },
        }
    }
}

// ---- helpers -------------------------------------------------------------

fn data_bits(n: u8) -> Result<DataBits, String> {
    DataBits::try_from(n).map_err(|e| e.to_string())
}

fn stop_bits(n: u8) -> Result<StopBits, String> {
    StopBits::try_from(n).map_err(|e| e.to_string())
}

fn timeout(ms: Option<u64>) -> Result<Duration, String> {
    let ms = ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    if ms > MAX_TIMEOUT_MS {
        return Err(format!(
            "timeout_ms must be at most {MAX_TIMEOUT_MS}; wait again to wait longer"
        ));
    }
    Ok(Duration::from_millis(ms))
}

fn compile(pattern: &str) -> Result<Regex, String> {
    regex::bytes::RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
        .map_err(|e| format!("invalid pattern: {e}"))
}

/// Parses `"s<digits>"` into a session id.
fn parse_session_id(text: &str) -> Option<SessionId> {
    let digits = text.strip_prefix('s')?;
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().map(SessionId)
}

/// Runs blocking core work off the async runtime.
async fn blocking<T, F>(work: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| format!("internal error: {e}"))?
}

fn port_out(info: PortInfo, sessions: &[SessionSummary]) -> PortOut {
    let session = sessions
        .iter()
        .find(|s| s.state == SessionState::Open && s.info.name == info.path)
        .map(|s| s.id.to_string());
    let mut out = PortOut {
        port: info.path,
        kind: String::new(),
        vid: None,
        pid: None,
        serial_number: None,
        manufacturer: None,
        product: None,
        session,
    };
    out.kind = match info.kind {
        PortKind::Usb {
            vid,
            pid,
            serial_number,
            manufacturer,
            product,
        } => {
            out.vid = Some(format!("{vid:04x}"));
            out.pid = Some(format!("{pid:04x}"));
            out.serial_number = serial_number;
            out.manufacturer = manufacturer;
            out.product = product;
            "usb"
        }
        PortKind::Bluetooth => "bluetooth",
        PortKind::Pci => "pci",
        PortKind::Unknown => "unknown",
    }
    .into();
    out
}

// ---- tools ---------------------------------------------------------------

#[tool_router]
impl SerialTools {
    /// Creates the tool set over `manager`, listing ports with `list_ports`.
    pub fn new(manager: Arc<SessionManager>, list_ports: Arc<PortLister>) -> Self {
        Self {
            manager,
            list_ports,
            tool_router: Self::tool_router(),
        }
    }

    /// Resolves a session id (`"s1"`) or alias. Ids win over aliases.
    fn session(&self, name: &str) -> Result<Arc<Session>, String> {
        if let Some(id) = parse_session_id(name)
            && let Ok(session) = self.manager.get(id)
        {
            return Ok(session);
        }
        self.manager
            .find_by_alias(name)
            .map_err(|_| format!("no session {name:?}; call session.list"))
    }

    #[tool(
        name = "session.list",
        description = "List the sessions (open or recently closed ports) with their id, alias, port and state.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    fn session_list(&self) -> Json<SessionsOut> {
        Json(SessionsOut {
            sessions: self.manager.list().into_iter().map(Into::into).collect(),
        })
    }

    #[tool(
        name = "port.list",
        description = "List the serial ports on this machine, with USB ids when known and the session already using each port.",
        annotations(read_only_hint = true)
    )]
    async fn port_list(&self) -> Result<Json<PortsOut>, String> {
        let lister = Arc::clone(&self.list_ports);
        let ports = blocking(move || lister().map_err(|e| e.to_string())).await?;
        let sessions = self.manager.list();
        Ok(Json(PortsOut {
            ports: ports.into_iter().map(|p| port_out(p, &sessions)).collect(),
        }))
    }

    #[tool(
        name = "session.open",
        description = "Open a serial port in a new session. Fails if the port is already open; reuse that session instead.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false
        )
    )]
    async fn session_open(
        &self,
        Parameters(args): Parameters<OpenArgs>,
    ) -> Result<Json<SessionOut>, String> {
        let mut config = SerialConfig::new(args.port, args.baud_rate);
        config.data_bits = data_bits(args.data_bits)?;
        config.stop_bits = stop_bits(args.stop_bits)?;
        config.parity = match args.parity {
            ParityArg::None => Parity::None,
            ParityArg::Odd => Parity::Odd,
            ParityArg::Even => Parity::Even,
        };
        config.flow_control = match args.flow_control {
            FlowControlArg::None => FlowControl::None,
            FlowControlArg::Software => FlowControl::Software,
            FlowControlArg::Hardware => FlowControl::Hardware,
        };
        if let Some(alias) = &args.alias
            && parse_session_id(alias).is_some()
        {
            return Err(format!("alias {alias:?} looks like a session id"));
        }
        let manager = Arc::clone(&self.manager);
        let id = blocking(move || {
            manager
                .open(&config, args.alias)
                .map(|s| s.id())
                .map_err(|e| e.to_string())
        })
        .await?;
        self.manager
            .list()
            .into_iter()
            .find(|s| s.id == id)
            .map(|s| Json(s.into()))
            .ok_or_else(|| format!("session {id} closed while opening"))
    }

    #[tool(
        name = "io.write",
        description = "Send bytes to a session. Returns the cursor taken just before sending; pass it to io.wait_for to wait for the reply.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn io_write(
        &self,
        Parameters(args): Parameters<WriteArgs>,
    ) -> Result<Json<WriteOut>, String> {
        let data = payload::outgoing(args.text.as_deref(), args.hex.as_deref())?;
        let session = self.session(&args.session)?;
        blocking(move || {
            let cursor = session.end();
            let seq = session.write(&data).map_err(|e| e.to_string())?;
            Ok(Json(WriteOut {
                bytes: data.len(),
                seq,
                cursor: cursor.0,
            }))
        })
        .await
    }

    #[tool(
        name = "io.read",
        description = "Read logged traffic (rx, tx and markers, with microsecond timestamps) from a cursor. Pass the returned cursor to the next read.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    fn io_read(&self, Parameters(args): Parameters<ReadArgs>) -> Result<Json<ReadOut>, String> {
        let budget = args.max_bytes.unwrap_or(DEFAULT_READ_BYTES);
        if budget == 0 || budget > MAX_READ_BYTES {
            return Err(format!("max_bytes must be between 1 and {MAX_READ_BYTES}"));
        }
        let session = self.session(&args.session)?;
        let result = session
            .read(Cursor(args.cursor), budget)
            .map_err(|e| e.to_string())?;
        let entries = result
            .entries
            .into_iter()
            .map(|e| {
                let (kind, data) = match e.kind {
                    EntryKind::Data { direction, bytes } => (
                        match direction {
                            Direction::Rx => "rx",
                            Direction::Tx => "tx",
                        },
                        payload::render(&bytes, args.format),
                    ),
                    EntryKind::Marker { label } => ("marker", label),
                };
                EntryOut {
                    seq: e.seq,
                    timestamp_us: e.timestamp_us,
                    kind: kind.into(),
                    data,
                }
            })
            .collect();
        Ok(Json(ReadOut {
            entries,
            cursor: result.next.0,
            missed: result.missed,
            more: result.next < session.end(),
        }))
    }

    #[tool(
        name = "io.wait_for",
        description = "Wait until received data matches a regular expression, or the timeout passes. Searches from `cursor` (default: data arriving after the call).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn io_wait_for(
        &self,
        Parameters(args): Parameters<WaitArgs>,
    ) -> Result<Json<WaitOut>, String> {
        let pattern = compile(&args.pattern)?;
        let timeout = timeout(args.timeout_ms)?;
        let session = self.session(&args.session)?;
        let format = args.format;
        blocking(move || {
            let from = args.cursor.map_or_else(|| session.end(), Cursor);
            let found = session
                .wait_for(&pattern, from, timeout, DEFAULT_MATCH_WINDOW)
                .map_err(|e| e.to_string())?;
            Ok(Json(WaitOut::from_result(found, from, format)))
        })
        .await
    }

    #[tool(
        name = "io.signals",
        description = "Set DTR and/or RTS (omit both to only read) and read CTS/DSR/RI/CD. DTR/RTS changes are logged as markers. Chip reset or boot-mode sequences are up to the caller.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true
        )
    )]
    async fn io_signals(
        &self,
        Parameters(args): Parameters<SignalsArgs>,
    ) -> Result<Json<SignalsOut>, String> {
        let session = self.session(&args.session)?;
        blocking(move || {
            if let Some(level) = args.dtr {
                session.set_dtr(level).map_err(|e| e.to_string())?;
            }
            if let Some(level) = args.rts {
                session.set_rts(level).map_err(|e| e.to_string())?;
            }
            let outputs = session.output_signals();
            let (lines, input_error) = match session.line_states() {
                Ok(l) => (Some(l), None),
                Err(e) => (None, Some(e.to_string())),
            };
            Ok(Json(SignalsOut {
                dtr: outputs.dtr,
                rts: outputs.rts,
                cts: lines.map(|l| l.cts),
                dsr: lines.map(|l| l.dsr),
                ri: lines.map(|l| l.ri),
                cd: lines.map(|l| l.cd),
                input_error,
            }))
        })
        .await
    }

    #[tool(
        name = "io.send_and_expect",
        description = "Send bytes, then wait until what is received afterwards matches a regular expression. Use for command/response exchanges.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn io_send_and_expect(
        &self,
        Parameters(args): Parameters<SendExpectArgs>,
    ) -> Result<Json<WaitOut>, String> {
        let data = payload::outgoing(args.text.as_deref(), args.hex.as_deref())?;
        let pattern = compile(&args.pattern)?;
        let timeout = timeout(args.timeout_ms)?;
        let session = self.session(&args.session)?;
        let format = args.format;
        blocking(move || {
            let from = session.end();
            let found = session
                .send_and_expect(&data, &pattern, timeout, DEFAULT_MATCH_WINDOW)
                .map_err(|e| e.to_string())?;
            Ok(Json(WaitOut::from_result(found, from, format)))
        })
        .await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SerialTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("serialx", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_parse_strictly() {
        assert_eq!(parse_session_id("s1"), Some(SessionId(1)));
        assert_eq!(parse_session_id("s42"), Some(SessionId(42)));
        for bad in ["", "s", "1", "S1", "s1a", "s-1", "s+1", "s99999999999"] {
            assert_eq!(parse_session_id(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn serial_settings_are_validated() {
        assert_eq!(data_bits(7).unwrap(), DataBits::Seven);
        assert_eq!(data_bits(5).unwrap(), DataBits::Five);
        assert_eq!(data_bits(6).unwrap(), DataBits::Six);
        assert!(data_bits(9).unwrap_err().contains("not 9"));
        assert_eq!(stop_bits(2).unwrap(), StopBits::Two);
        assert!(stop_bits(0).is_err());
    }

    #[test]
    fn timeouts_have_a_default_and_a_cap() {
        assert_eq!(timeout(None).unwrap(), Duration::from_secs(5));
        assert_eq!(timeout(Some(0)).unwrap(), Duration::ZERO);
        assert_eq!(
            timeout(Some(MAX_TIMEOUT_MS)).unwrap(),
            Duration::from_secs(60)
        );
        assert!(timeout(Some(MAX_TIMEOUT_MS + 1)).is_err());
    }

    #[test]
    fn patterns_compile_with_errors_reported() {
        assert!(compile(r"OK\r?\n").unwrap().is_match(b"OK\r\n"));
        assert!(compile("(").unwrap_err().starts_with("invalid pattern"));
        assert!(compile(r"\w{100000}").is_err(), "size limit applies");
    }

    #[test]
    fn port_kinds_are_rendered() {
        let usb = PortInfo {
            path: "COM3".into(),
            kind: PortKind::Usb {
                vid: 0x1a86,
                pid: 0x7523,
                serial_number: Some("A1".into()),
                manufacturer: Some("wch.cn".into()),
                product: None,
            },
        };
        let out = port_out(usb, &[]);
        assert_eq!(out.kind, "usb");
        assert_eq!(out.vid.as_deref(), Some("1a86"));
        assert_eq!(out.pid.as_deref(), Some("7523"));
        assert_eq!(out.serial_number.as_deref(), Some("A1"));
        assert_eq!(out.manufacturer.as_deref(), Some("wch.cn"));
        assert_eq!(out.session, None);
        for (kind, name) in [
            (PortKind::Bluetooth, "bluetooth"),
            (PortKind::Pci, "pci"),
            (PortKind::Unknown, "unknown"),
        ] {
            let out = port_out(
                PortInfo {
                    path: "x".into(),
                    kind,
                },
                &[],
            );
            assert_eq!(out.kind, name);
            assert_eq!(out.vid, None);
        }
    }
}
