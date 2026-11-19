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
use serialbot_core::transport::serial::{
    DataBits, FlowControl, Parity, PortInfo, PortKind, SerialConfig, StopBits,
};
use serialbot_core::transport::{TransportError, TransportKind};
use serialbot_core::{
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

const INSTRUCTIONS: &str = "Drives serial ports and network endpoints opened in SerialBot. \
Use port.list to find ports, session.open to open one (or session.list to \
reuse an open session), then io.write / io.read / io.wait_for / \
io.send_and_expect, and io.signals for DTR/RTS. Every log entry has a sequence number; pass the \
`cursor` a result returns to the next call so nothing is read twice. \
io.write returns the cursor taken just before sending: pass it to \
io.wait_for to catch replies that arrive very fast. Before running an external \
tool on the same port (a flasher), call session.release, then session.reopen \
afterwards: the log continues. log.mark sets a named point and log.query \
summarizes or searches what happened after it. command.list shows further \
commands, run with command.call.";

/// Longest `session.reopen` wait for a device to come back.
pub(crate) const MAX_REOPEN_WAIT_MS: u64 = 60_000;
/// Pause between `session.reopen` attempts.
const REOPEN_RETRY: Duration = Duration::from_millis(200);
/// Default payload budget of `log.query` results.
pub(crate) const DEFAULT_QUERY_BYTES: usize = 16 * 1024;

/// MCP handler exposing serialbot sessions as tools.
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
    /// "/dev/ttyUSB0"; or a network endpoint: "tcp://host:port",
    /// "tcp-server://addr:port" (one client at a time) or
    /// "udp://host:port?local=addr:port". Line settings are ignored for
    /// network endpoints.
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

/// Arguments naming one session.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct SessionArgs {
    /// Session id or alias.
    pub session: String,
}

/// Arguments of `session.reopen`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ReopenArgs {
    /// Session id or alias.
    pub session: String,
    /// Keep trying this long (ms, max 60000) while the device is not back
    /// yet, e.g. while a board re-enumerates after flashing. Default 0: try
    /// once.
    #[serde(default)]
    pub wait_ms: u64,
}

/// Arguments of `session.configure`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct ConfigureArgs {
    /// Session id or alias.
    pub session: String,
    /// New baud rate, applied without closing the port and kept for
    /// reopening.
    pub baud_rate: u32,
}

/// Arguments of `log.mark`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct MarkArgs {
    /// Session id or alias.
    pub session: String,
    /// Label of the marker, e.g. "reset" or "test 3 start".
    pub label: String,
}

/// Arguments of `log.query`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct QueryArgs {
    /// Session id or alias.
    pub session: String,
    /// Start after the latest marker whose label contains this text.
    #[serde(default)]
    pub after_marker: Option<String>,
    /// Start at this cursor instead (default: the oldest entry kept).
    #[serde(default)]
    pub cursor: Option<u64>,
    /// Only return entries whose text (data or marker label) matches this
    /// regular expression. The summary still counts everything.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Payload budget of the returned entries (default 16384, max 1048576).
    #[serde(default)]
    pub max_bytes: Option<usize>,
    /// How data is rendered (default text).
    #[serde(default)]
    pub format: Format,
}

/// Arguments of `command.call`.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct CallArgs {
    /// Command name from command.list.
    pub name: String,
    /// Arguments as described by command.list.
    #[serde(default)]
    pub args: serde_json::Value,
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
                TransportKind::TcpClient => "tcp",
                TransportKind::TcpServer => "tcp-server",
                TransportKind::Udp => "udp",
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

/// Result of `log.mark`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct MarkOut {
    /// Sequence number of the marker.
    pub seq: u64,
    /// Cursor just after the marker: read or wait from here.
    pub cursor: u64,
}

/// Counts over the queried range.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub(crate) struct QuerySummary {
    /// Entries in the range.
    pub entries: u64,
    /// Bytes received.
    pub rx_bytes: u64,
    /// Bytes sent.
    pub tx_bytes: u64,
    /// Entries matching `pattern` (all entries without one).
    pub matches: u64,
    /// Marker labels in order (at most 50).
    pub markers: Vec<String>,
    /// Time of the first entry (µs since the Unix epoch).
    pub first_us: Option<u64>,
    /// Time of the last entry.
    pub last_us: Option<u64>,
}

/// Result of `log.query`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct QueryOut {
    /// Matching entries, oldest first, within the payload budget.
    pub entries: Vec<EntryOut>,
    /// Whether matching entries were left out because of the budget.
    pub truncated: bool,
    /// Counts over the whole range.
    pub summary: QuerySummary,
    /// Cursor just past the range: continue from here.
    pub cursor: u64,
    /// Entries already dropped from the log before the start.
    pub missed: u64,
}

/// A command in `command.list`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct CommandOut {
    /// Name to pass to command.call.
    pub name: String,
    /// What it does.
    pub description: String,
    /// Its arguments, as an example object.
    pub args: serde_json::Value,
}

/// Result of `command.list`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct CommandsOut {
    /// Available commands.
    pub commands: Vec<CommandOut>,
}

/// Result of `command.call`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct CallOut {
    /// What the command returned.
    pub result: serde_json::Value,
}

/// Result of `io.signals`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct SignalsOut {
    /// DTR level last set in this session (null: never set).
    pub dtr: Option<bool>,
    /// RTS level last set in this session (null: never set).
    pub rts: Option<bool>,
    /// False while RTS/CTS hardware flow control drives RTS; setting it is
    /// then refused.
    pub rts_manual: bool,
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

fn entry_out(e: serialbot_core::Entry, format: Format) -> EntryOut {
    let (kind, data) = match e.kind {
        EntryKind::Data { direction, bytes } => (
            match direction {
                Direction::Rx => "rx",
                Direction::Tx => "tx",
            },
            payload::render(&bytes, format),
        ),
        EntryKind::Marker { label } => ("marker", label),
    };
    EntryOut {
        seq: e.seq,
        timestamp_us: e.timestamp_us,
        kind: kind.into(),
        data,
    }
}

/// Extra commands reached through `command.call`, with example arguments.
const COMMANDS: &[(&str, &str, &str)] = &[
    (
        "session.rename",
        "Set or clear a session's alias.",
        r#"{"session":"s1","alias":"esp32"}"#,
    ),
    (
        "session.auto_reconnect",
        "Turn reopening the port by itself after the device comes back on or off.",
        r#"{"session":"s1","on":true}"#,
    ),
    (
        "io.break",
        "Hold a BREAK condition on the line for ms milliseconds (1 to 5000, default 250).",
        r#"{"session":"s1","ms":250}"#,
    ),
];

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
            tool_router: Self::tool_router() + Self::more_tools(),
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

    /// The summary of one session as a tool result.
    fn summary(&self, id: SessionId) -> Result<Json<SessionOut>, String> {
        self.manager
            .list()
            .into_iter()
            .find(|s| s.id == id)
            .map(|s| Json(s.into()))
            .ok_or_else(|| format!("no session {id}"))
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
        description = "Open a serial port, or a network endpoint (tcp://host:port, tcp-server://addr:port, udp://host:port?local=addr:port), in a new session. Fails if it is already open; reuse that session instead.",
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
            .map(|e| entry_out(e, args.format))
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
                rts_manual: session.info().capabilities.manual_rts,
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

#[tool_router(router = more_tools, vis = "pub(crate)")]
impl SerialTools {
    #[tool(
        name = "session.close",
        description = "Close a session's port. The session stays listed and its log readable; it cannot be reopened (open the port again for a new session).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true
        )
    )]
    async fn session_close(
        &self,
        Parameters(args): Parameters<SessionArgs>,
    ) -> Result<Json<SessionOut>, String> {
        let id = self.session(&args.session)?.id();
        let manager = Arc::clone(&self.manager);
        blocking(move || manager.close(id).map_err(|e| e.to_string())).await?;
        self.summary(id)
    }

    #[tool(
        name = "session.release",
        description = "Hand the port back to the operating system so another program (a flasher, a terminal) can open it, keeping the session and its log. Reopen it with session.reopen; it is not reopened automatically before that.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    async fn session_release(
        &self,
        Parameters(args): Parameters<SessionArgs>,
    ) -> Result<Json<SessionOut>, String> {
        let id = self.session(&args.session)?.id();
        let manager = Arc::clone(&self.manager);
        blocking(move || manager.release(id).map_err(|e| e.to_string())).await?;
        self.summary(id)
    }

    #[tool(
        name = "session.reopen",
        description = "Reopen the port of a released, disconnected or failed session with its settings; the log continues. With wait_ms, keep trying while the device is not back yet (e.g. a board re-enumerating after flashing).",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    async fn session_reopen(
        &self,
        Parameters(args): Parameters<ReopenArgs>,
    ) -> Result<Json<SessionOut>, String> {
        if args.wait_ms > MAX_REOPEN_WAIT_MS {
            return Err(format!("wait_ms must be at most {MAX_REOPEN_WAIT_MS}"));
        }
        let id = self.session(&args.session)?.id();
        let manager = Arc::clone(&self.manager);
        let wait = Duration::from_millis(args.wait_ms);
        blocking(move || {
            let deadline = std::time::Instant::now() + wait;
            loop {
                match manager.reconnect(id) {
                    Ok(()) => return Ok(()),
                    // Only a device that is not there (yet) is worth waiting for.
                    Err(serialbot_core::ManagerError::Session(
                        serialbot_core::SessionError::Transport(TransportError::NotFound(_)),
                    )) if std::time::Instant::now() + REOPEN_RETRY <= deadline => {
                        std::thread::sleep(REOPEN_RETRY);
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
        })
        .await?;
        self.summary(id)
    }

    #[tool(
        name = "session.configure",
        description = "Change a session's baud rate without closing the port. The new rate is also used when the session is reopened.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true
        )
    )]
    async fn session_configure(
        &self,
        Parameters(args): Parameters<ConfigureArgs>,
    ) -> Result<Json<SessionOut>, String> {
        let id = self.session(&args.session)?.id();
        let manager = Arc::clone(&self.manager);
        blocking(move || {
            manager
                .set_baud_rate(id, args.baud_rate)
                .map_err(|e| e.to_string())
        })
        .await?;
        self.summary(id)
    }

    #[tool(
        name = "log.mark",
        description = "Insert a named marker into a session's log, e.g. before resetting a board, so log.query can look at what happened after it.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    fn log_mark(&self, Parameters(args): Parameters<MarkArgs>) -> Result<Json<MarkOut>, String> {
        if args.label.trim().is_empty() {
            return Err("label must not be empty".into());
        }
        let session = self.session(&args.session)?;
        let seq = session.mark(&args.label);
        Ok(Json(MarkOut {
            seq,
            cursor: seq + 1,
        }))
    }

    #[tool(
        name = "log.query",
        description = "Summarize and search a session's log: from a cursor or after the latest marker with a label, count rx/tx bytes and markers, and return the entries matching an optional regular expression.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn log_query(
        &self,
        Parameters(args): Parameters<QueryArgs>,
    ) -> Result<Json<QueryOut>, String> {
        let budget = args.max_bytes.unwrap_or(DEFAULT_QUERY_BYTES);
        if budget == 0 || budget > MAX_READ_BYTES {
            return Err(format!("max_bytes must be between 1 and {MAX_READ_BYTES}"));
        }
        let pattern = args.pattern.as_deref().map(compile).transpose()?;
        let session = self.session(&args.session)?;
        blocking(move || query(&session, &args, pattern.as_ref(), budget)).await
    }

    #[tool(
        name = "command.list",
        description = "List further commands (beyond the tools) with their arguments; run one with command.call.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    #[allow(clippy::unused_self, reason = "tool handlers are methods")]
    fn command_list(&self) -> Json<CommandsOut> {
        Json(CommandsOut {
            commands: COMMANDS
                .iter()
                .map(|(name, description, args)| CommandOut {
                    name: (*name).into(),
                    description: (*description).into(),
                    args: serde_json::from_str(args).unwrap_or_default(),
                })
                .collect(),
        })
    }

    #[tool(
        name = "command.call",
        description = "Run a command from command.list with its arguments.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = false
        )
    )]
    async fn command_call(
        &self,
        Parameters(call): Parameters<CallArgs>,
    ) -> Result<Json<CallOut>, String> {
        let arg = |key: &str| call.args.get(key).cloned().unwrap_or_default();
        let session_name = arg("session")
            .as_str()
            .map(str::to_owned)
            .ok_or("args.session is required; see command.list")?;
        let session = self.session(&session_name)?;
        let id = session.id();
        let result = match call.name.as_str() {
            "session.rename" => {
                let alias = match arg("alias") {
                    serde_json::Value::Null => None,
                    serde_json::Value::String(a) if !a.is_empty() => {
                        if parse_session_id(&a).is_some() {
                            return Err(format!("alias {a:?} looks like a session id"));
                        }
                        Some(a)
                    }
                    _ => return Err("args.alias must be a non-empty string or null".into()),
                };
                self.manager
                    .set_alias(id, alias.clone())
                    .map_err(|e| e.to_string())?;
                serde_json::json!({ "alias": alias })
            }
            "session.auto_reconnect" => {
                let on = arg("on").as_bool().ok_or("args.on must be true or false")?;
                self.manager
                    .set_auto_reconnect(id, on)
                    .map_err(|e| e.to_string())?;
                serde_json::json!({ "auto_reconnect": on })
            }
            "io.break" => {
                let ms = match arg("ms") {
                    serde_json::Value::Null => 250,
                    v => v
                        .as_u64()
                        .filter(|ms| (1..=5000).contains(ms))
                        .ok_or("args.ms must be a whole number of milliseconds from 1 to 5000")?,
                };
                blocking(move || {
                    session
                        .send_break(Duration::from_millis(ms))
                        .map_err(|e| e.to_string())
                })
                .await?;
                serde_json::json!({ "break_ms": ms })
            }
            other => return Err(format!("unknown command {other:?}; see command.list")),
        };
        Ok(Json(CallOut { result }))
    }
}

/// Scans the log for `log.query`.
fn query(
    session: &Session,
    args: &QueryArgs,
    pattern: Option<&Regex>,
    budget: usize,
) -> Result<Json<QueryOut>, String> {
    let read_all = |from: Cursor| -> Result<serialbot_core::ReadResult, String> {
        session.read(from, usize::MAX).map_err(|e| e.to_string())
    };
    let start = match (&args.after_marker, args.cursor) {
        (Some(_), Some(_)) => return Err("give either after_marker or cursor, not both".into()),
        (Some(label), None) => {
            let all = read_all(Cursor(0))?;
            let marker = all
                .entries
                .iter()
                .rev()
                .find(|e| matches!(&e.kind, EntryKind::Marker { label: l } if l.contains(label.as_str())))
                .ok_or_else(|| format!("no marker containing {label:?} in the log"))?;
            Cursor(marker.seq + 1)
        }
        (None, cursor) => Cursor(cursor.unwrap_or(0)),
    };
    let result = read_all(start)?;
    let mut out = QueryOut {
        entries: Vec::new(),
        truncated: false,
        summary: QuerySummary::default(),
        cursor: result.next.0,
        missed: result.missed,
    };
    let mut used = 0;
    for e in result.entries {
        let s = &mut out.summary;
        s.entries += 1;
        s.first_us.get_or_insert(e.timestamp_us);
        s.last_us = Some(e.timestamp_us);
        let hit = match &e.kind {
            EntryKind::Data { direction, bytes } => {
                match direction {
                    Direction::Rx => s.rx_bytes += bytes.len() as u64,
                    Direction::Tx => s.tx_bytes += bytes.len() as u64,
                }
                pattern.is_none_or(|p| p.is_match(bytes))
            }
            EntryKind::Marker { label } => {
                if s.markers.len() < 50 {
                    s.markers.push(label.clone());
                }
                pattern.is_none_or(|p| p.is_match(label.as_bytes()))
            }
        };
        if !hit {
            continue;
        }
        s.matches += 1;
        let size = match &e.kind {
            EntryKind::Data { bytes, .. } => bytes.len(),
            EntryKind::Marker { label } => label.len(),
        };
        // At least one entry, then as many as fit.
        if used > 0 && used + size > budget {
            out.truncated = true;
            continue;
        }
        used += size;
        out.entries.push(entry_out(e, args.format));
    }
    Ok(Json(out))
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for SerialTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("serialbot", env!("CARGO_PKG_VERSION")))
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
