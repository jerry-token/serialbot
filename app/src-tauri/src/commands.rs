//! What the frontend can ask the backend to do.
//!
//! [`AppState`] holds the plain-Rust implementation so it can be tested
//! with mock devices; the `#[tauri::command]` functions in `lib.rs` only
//! forward to it. Results are serialized with camelCase field names for
//! TypeScript, and errors are user-facing messages.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serialbot_core::transport::TransportError;
use serialbot_core::transport::serial::{
    DataBits, FlowControl, Parity, PortInfo, PortKind, SerialConfig, StopBits,
};
use serialbot_core::{
    Cursor, Direction, EntryKind, Session, SessionId, SessionManager, SessionState, SessionSummary,
};
pub use serialbot_mcp::PortLister;

use crate::export::{CaptureStatus, Captures, ExportFormat, ExportSummary};
use crate::filesend::{FileSendStatus, FileSends};
use crate::repeat::{RepeatStatus, Repeaters};

/// Largest payload one [`AppState::read`] returns.
pub const MAX_READ_BYTES: usize = 4 * 1024 * 1024;

/// A port in the port picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortDto {
    /// OS name to open.
    pub path: String,
    /// `"usb"`, `"bluetooth"`, `"pci"` or `"unknown"`.
    pub kind: &'static str,
    /// USB vendor id.
    pub vid: Option<u16>,
    /// USB product id.
    pub pid: Option<u16>,
    /// USB manufacturer string.
    pub manufacturer: Option<String>,
    /// USB product string.
    pub product: Option<String>,
    /// USB serial number.
    pub serial_number: Option<String>,
}

impl From<PortInfo> for PortDto {
    fn from(info: PortInfo) -> Self {
        let mut dto = Self {
            path: info.path,
            kind: "unknown",
            vid: None,
            pid: None,
            manufacturer: None,
            product: None,
            serial_number: None,
        };
        match info.kind {
            PortKind::Usb {
                vid,
                pid,
                serial_number,
                manufacturer,
                product,
            } => {
                dto.kind = "usb";
                dto.vid = Some(vid);
                dto.pid = Some(pid);
                dto.manufacturer = manufacturer;
                dto.product = product;
                dto.serial_number = serial_number;
            }
            PortKind::Bluetooth => dto.kind = "bluetooth",
            PortKind::Pci => dto.kind = "pci",
            PortKind::Unknown => {}
        }
        dto
    }
}

/// Parity as sent by the frontend.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ParityArg {
    /// No parity bit.
    #[default]
    None,
    /// Odd parity.
    Odd,
    /// Even parity.
    Even,
}

/// Flow control as sent by the frontend.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlowControlArg {
    /// No flow control.
    #[default]
    None,
    /// XON/XOFF.
    Software,
    /// RTS/CTS.
    Hardware,
}

/// Settings of a port to open. Missing fields default to 115200 8N1 with
/// no flow control.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenRequest {
    /// OS name of the port.
    pub port: String,
    /// Baud rate.
    pub baud_rate: u32,
    /// Data bits, 5 to 8.
    pub data_bits: u8,
    /// Parity.
    pub parity: ParityArg,
    /// Stop bits, 1 or 2.
    pub stop_bits: u8,
    /// Flow control.
    pub flow_control: FlowControlArg,
    /// Reopen the port automatically when the device comes back after an
    /// unplug or a USB re-enumeration (default on).
    pub auto_reconnect: bool,
}

impl Default for OpenRequest {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud_rate: 115_200,
            data_bits: 8,
            parity: ParityArg::None,
            stop_bits: 1,
            flow_control: FlowControlArg::None,
            auto_reconnect: true,
        }
    }
}

impl OpenRequest {
    /// Converts to core settings, rejecting invalid bit counts.
    fn to_config(&self) -> Result<SerialConfig, TransportError> {
        let mut config = SerialConfig::new(self.port.clone(), self.baud_rate);
        config.data_bits = DataBits::try_from(self.data_bits)?;
        config.stop_bits = StopBits::try_from(self.stop_bits)?;
        config.parity = match self.parity {
            ParityArg::None => Parity::None,
            ParityArg::Odd => Parity::Odd,
            ParityArg::Even => Parity::Even,
        };
        config.flow_control = match self.flow_control {
            FlowControlArg::None => FlowControl::None,
            FlowControlArg::Software => FlowControl::Software,
            FlowControlArg::Hardware => FlowControl::Hardware,
        };
        Ok(config)
    }
}

/// A session as shown by the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDto {
    /// Session number (`1` for "s1").
    pub id: u32,
    /// Alias given by an MCP client, if any.
    pub alias: Option<String>,
    /// Port name.
    pub port: String,
    /// `"open"`, `"disconnected"`, `"closed"` or `"failed"`.
    pub state: &'static str,
    /// Reason, when the state is `"failed"`.
    pub error: Option<String>,
    /// Whether the port reopens automatically when the device comes back.
    pub auto_reconnect: bool,
    /// Whether AI agents (MCP) may use the session.
    pub ai_access: bool,
}

/// Converts a manager listing entry.
pub fn session_dto(summary: &SessionSummary) -> SessionDto {
    let (state, error) = state_parts(summary.state.clone());
    SessionDto {
        id: summary.id.0,
        alias: summary.alias.clone(),
        port: summary.info.name.clone(),
        state,
        error,
        auto_reconnect: summary.auto_reconnect,
        ai_access: summary.ai_access,
    }
}

fn state_parts(state: SessionState) -> (&'static str, Option<String>) {
    match state {
        SessionState::Open => ("open", None),
        SessionState::Disconnected => ("disconnected", None),
        SessionState::Closed => ("closed", None),
        SessionState::Failed(reason) => ("failed", Some(reason)),
    }
}

/// One log entry for the frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntryDto {
    /// Sequence number.
    pub seq: u64,
    /// Microseconds since the Unix epoch.
    pub timestamp_us: u64,
    /// `"rx"`, `"tx"` or `"marker"`.
    pub kind: &'static str,
    /// The bytes of an rx/tx entry; empty for markers.
    pub data: Vec<u8>,
    /// The label of a marker.
    pub label: Option<String>,
    /// Whether an AI agent sent the data or set the marker.
    pub ai: bool,
}

/// Result of [`AppState::read`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogChunk {
    /// Entries in order.
    pub entries: Vec<LogEntryDto>,
    /// Cursor for the next read.
    pub next: u64,
    /// Entries already dropped from the log before they could be read.
    pub missed: u64,
    /// Session state at the time of the read, so the UI notices unplugs.
    pub state: &'static str,
    /// Reason, when the state is `"failed"`.
    pub error: Option<String>,
}

/// Modem line levels of a session for the status bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalsDto {
    /// Session number.
    pub id: u32,
    /// DTR level last set (`null`: not set in this session).
    pub dtr: Option<bool>,
    /// RTS level last set (`null`: not set in this session).
    pub rts: Option<bool>,
    /// Whether RTS can be set by hand (false under RTS/CTS flow control).
    pub rts_manual: bool,
    /// Clear To Send (`null`: the port cannot report input lines).
    pub cts: Option<bool>,
    /// Data Set Ready.
    pub dsr: Option<bool>,
    /// Ring Indicator.
    pub ri: Option<bool>,
    /// Carrier Detect.
    pub cd: Option<bool>,
}

/// Reads the line levels of `session`; input lines are `None` when they
/// cannot be read (closed session, PTY, unsupported adapter).
pub fn signals_dto(session: &Session) -> SignalsDto {
    let outputs = session.output_signals();
    let lines = session.line_states().ok();
    SignalsDto {
        id: session.id().0,
        dtr: outputs.dtr,
        rts: outputs.rts,
        rts_manual: session.info().capabilities.manual_rts,
        cts: lines.map(|l| l.cts),
        dsr: lines.map(|l| l.dsr),
        ri: lines.map(|l| l.ri),
        cd: lines.map(|l| l.cd),
    }
}

/// Backend state shared by all windows.
pub struct AppState {
    manager: Arc<SessionManager>,
    list_ports: Arc<PortLister>,
    repeaters: Repeaters,
    captures: Captures,
    file_sends: FileSends,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("manager", &self.manager)
            .finish_non_exhaustive()
    }
}

impl AppState {
    /// State over `manager`, listing ports with `list_ports`.
    pub fn new(manager: Arc<SessionManager>, list_ports: Arc<PortLister>) -> Self {
        Self {
            manager,
            list_ports,
            repeaters: Repeaters::default(),
            captures: Captures::default(),
            file_sends: FileSends::default(),
        }
    }

    /// State over the machine's real serial ports.
    pub fn with_serial_ports() -> Self {
        Self::new(
            Arc::new(SessionManager::with_serial_ports()),
            Arc::new(serialbot_core::transport::serial::list_ports),
        )
    }

    /// The session manager, shared with the MCP server.
    pub fn manager(&self) -> &Arc<SessionManager> {
        &self.manager
    }

    /// The port lister, shared with the MCP server.
    pub fn port_lister(&self) -> &Arc<PortLister> {
        &self.list_ports
    }

    /// All sessions, oldest first, including those opened over MCP.
    pub fn sessions(&self) -> Vec<SessionDto> {
        self.manager.list().iter().map(session_dto).collect()
    }

    /// Ports present on the machine.
    pub fn ports(&self) -> Result<Vec<PortDto>, String> {
        (self.list_ports)()
            .map(|ports| ports.into_iter().map(Into::into).collect())
            .map_err(|e| e.to_string())
    }

    /// Opens a port in a new session.
    pub fn open(&self, request: &OpenRequest) -> Result<SessionDto, String> {
        let config = request.to_config().map_err(|e| e.to_string())?;
        let session = self
            .manager
            .open(&config, None)
            .map_err(|e| e.to_string())?;
        self.manager
            .set_auto_reconnect(session.id(), request.auto_reconnect)
            .map_err(|e| e.to_string())?;
        self.manager
            .list()
            .iter()
            .find(|s| s.id == session.id())
            .map(session_dto)
            .ok_or_else(|| "the session closed while opening".to_owned())
    }

    /// Reopens a disconnected or failed session's port now (same session,
    /// same log). Does nothing for an open session.
    pub fn reconnect(&self, id: u32) -> Result<(), String> {
        self.manager
            .reconnect(SessionId(id))
            .map_err(|e| e.to_string())
    }

    /// Turns automatic reconnecting of a session on or off.
    pub fn set_auto_reconnect(&self, id: u32, on: bool) -> Result<(), String> {
        self.manager
            .set_auto_reconnect(SessionId(id), on)
            .map_err(|e| e.to_string())
    }

    /// Lets AI agents use the session through MCP, or withholds it.
    pub fn set_ai_access(&self, id: u32, allowed: bool) -> Result<(), String> {
        self.manager
            .set_ai_access(SessionId(id), allowed)
            .map_err(|e| e.to_string())
    }

    /// Closes a session and forgets it; the frontend keeps what it shows.
    pub fn close(&self, id: u32) -> Result<(), String> {
        self.repeaters.stop(id);
        self.file_sends.remove(id);
        self.captures.remove(id);
        self.manager
            .remove(SessionId(id))
            .map_err(|e| e.to_string())
    }

    /// Sends bytes; returns the sequence number of the logged TX entry.
    pub fn write(&self, id: u32, data: &[u8]) -> Result<u64, String> {
        if data.is_empty() {
            return Err("nothing to send".into());
        }
        let session = self.manager.get(SessionId(id)).map_err(|e| e.to_string())?;
        session.write(data).map_err(|e| e.to_string())
    }

    /// Reads log entries from `cursor`, up to `max_bytes` of payload
    /// (clamped to 1..=[`MAX_READ_BYTES`]). Returns no entries when nothing
    /// new has arrived.
    pub fn read(&self, id: u32, cursor: u64, max_bytes: usize) -> Result<LogChunk, String> {
        let session = self.manager.get(SessionId(id)).map_err(|e| e.to_string())?;
        let (state, error) = state_parts(session.state());
        let result = session
            .read(Cursor(cursor), max_bytes.clamp(1, MAX_READ_BYTES))
            .map_err(|e| e.to_string())?;
        let entries = result
            .entries
            .into_iter()
            .map(|e| {
                let (kind, data, label) = match e.kind {
                    EntryKind::Data { direction, bytes } => (
                        match direction {
                            Direction::Rx => "rx",
                            Direction::Tx => "tx",
                        },
                        bytes.to_vec(),
                        None,
                    ),
                    EntryKind::Marker { label } => ("marker", Vec::new(), Some(label)),
                };
                LogEntryDto {
                    seq: e.seq,
                    timestamp_us: e.timestamp_us,
                    kind,
                    data,
                    label,
                    ai: e.actor == serialbot_core::Actor::Ai,
                }
            })
            .collect();
        Ok(LogChunk {
            entries,
            next: result.next.0,
            missed: result.missed,
            state,
            error,
        })
    }

    /// Current line levels of a session.
    pub fn signals(&self, id: u32) -> Result<SignalsDto, String> {
        let session = self.manager.get(SessionId(id)).map_err(|e| e.to_string())?;
        Ok(signals_dto(&session))
    }

    /// Drives DTR and/or RTS (`None` leaves a line unchanged), then returns
    /// the line levels.
    pub fn set_signals(
        &self,
        id: u32,
        dtr: Option<bool>,
        rts: Option<bool>,
    ) -> Result<SignalsDto, String> {
        let session = self.manager.get(SessionId(id)).map_err(|e| e.to_string())?;
        if let Some(level) = dtr {
            session.set_dtr(level).map_err(|e| e.to_string())?;
        }
        if let Some(level) = rts {
            session.set_rts(level).map_err(|e| e.to_string())?;
        }
        Ok(signals_dto(&session))
    }

    /// Encodes typed text for sending in a non-UTF-8 encoding (WHATWG
    /// label such as `gbk`); fails on unknown labels and characters the
    /// encoding lacks.
    pub fn encode_text(&self, text: &str, encoding: &str) -> Result<Vec<u8>, String> {
        serialbot_core::encoding::encode_text(text, encoding).map_err(|e| e.to_string())
    }

    /// Closes every session (on app exit).
    pub fn shutdown(&self) {
        self.repeaters.stop_all();
        self.file_sends.stop_all();
        self.captures.stop_all();
        self.manager.close_all();
    }

    /// Saves what the session's log holds now to `path` (replacing it).
    /// `utc_offset_min` is the local time zone, minutes east of UTC.
    pub fn export_log(
        &self,
        id: u32,
        path: &str,
        format: ExportFormat,
        utc_offset_min: i32,
    ) -> Result<ExportSummary, String> {
        let session = self.manager.get(SessionId(id)).map_err(|e| e.to_string())?;
        crate::export::export(&session, std::path::Path::new(path), format, utc_offset_min)
    }

    /// Starts saving the session's log to `path` as it grows, beginning
    /// with what it already holds.
    pub fn start_capture(
        &self,
        id: u32,
        path: &str,
        format: ExportFormat,
        utc_offset_min: i32,
    ) -> Result<CaptureStatus, String> {
        let session = self.manager.get(SessionId(id)).map_err(|e| e.to_string())?;
        self.captures.start(
            id,
            session,
            std::path::Path::new(path),
            format,
            utc_offset_min,
        )
    }

    /// Stops the session's live capture, keeping its final status.
    pub fn stop_capture(&self, id: u32) -> CaptureStatus {
        self.captures.stop(id);
        self.captures.status(id)
    }

    /// State of the session's live capture.
    pub fn capture_status(&self, id: u32) -> CaptureStatus {
        self.captures.status(id)
    }

    /// Starts sending `data` every `interval_ms` (`count` times, or until
    /// stopped), replacing the session's previous repeat job.
    pub fn start_repeat(
        &self,
        id: u32,
        data: Vec<u8>,
        interval_ms: u64,
        count: Option<u32>,
    ) -> Result<RepeatStatus, String> {
        let session = self.manager.get(SessionId(id)).map_err(|e| e.to_string())?;
        self.repeaters
            .start(id, session, data, interval_ms, count)?;
        Ok(self.repeaters.status(id))
    }

    /// Stops the session's repeat job, if any.
    pub fn stop_repeat(&self, id: u32) -> RepeatStatus {
        self.repeaters.stop(id);
        self.repeaters.status(id)
    }

    /// Progress of the session's repeat job.
    pub fn repeat_status(&self, id: u32) -> RepeatStatus {
        self.repeaters.status(id)
    }

    /// Starts writing the file at `path` in chunks of `chunk` bytes with
    /// `delay_ms` after each, replacing the session's previous file job
    /// (see [`FileSends::start`] for the limits).
    pub fn send_file(
        &self,
        id: u32,
        path: &str,
        chunk: usize,
        delay_ms: u64,
    ) -> Result<FileSendStatus, String> {
        let session = self.manager.get(SessionId(id)).map_err(|e| e.to_string())?;
        self.file_sends
            .start(id, session, std::path::Path::new(path), chunk, delay_ms)
    }

    /// Stops the session's file job, if any; returns its final progress.
    pub fn stop_file_send(&self, id: u32) -> FileSendStatus {
        self.file_sends.stop(id);
        self.file_sends.status(id)
    }

    /// Progress of the session's current or last file job.
    pub fn file_send_status(&self, id: u32) -> FileSendStatus {
        self.file_sends.status(id)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use serialbot_core::transport::mock::{self, MockDevice};
    use serialbot_core::{SessionOptions, SystemClock};

    use super::*;

    type Devices = Arc<Mutex<HashMap<String, MockDevice>>>;

    fn state() -> (AppState, Devices) {
        let devices: Devices = Arc::default();
        let registry = Arc::clone(&devices);
        let manager = SessionManager::new(
            Box::new(move |config: &SerialConfig| {
                if !config.path.starts_with("mock") {
                    return Err(TransportError::NotFound(config.path.clone()));
                }
                let (connection, device) = mock::connection(&config.path, Duration::from_millis(5));
                registry.lock().unwrap().insert(config.path.clone(), device);
                Ok(connection)
            }),
            Arc::new(SystemClock),
            SessionOptions::default(),
        );
        let lister = || {
            Ok(vec![
                PortInfo {
                    path: "mock0".into(),
                    kind: PortKind::Usb {
                        vid: 0x10c4,
                        pid: 0xea60,
                        serial_number: Some("0001".into()),
                        manufacturer: Some("Silicon Labs".into()),
                        product: Some("CP2102".into()),
                    },
                },
                PortInfo {
                    path: "bt".into(),
                    kind: PortKind::Bluetooth,
                },
                PortInfo {
                    path: "pci".into(),
                    kind: PortKind::Pci,
                },
                PortInfo {
                    path: "pty".into(),
                    kind: PortKind::Unknown,
                },
            ])
        };
        (AppState::new(Arc::new(manager), Arc::new(lister)), devices)
    }

    fn request(port: &str) -> OpenRequest {
        OpenRequest {
            port: port.into(),
            ..OpenRequest::default()
        }
    }

    /// Reads until at least `count` entries arrived since `cursor`.
    fn read_entries(app: &AppState, id: u32, count: usize) -> Vec<LogEntryDto> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut entries = Vec::new();
        let mut cursor = 0;
        while entries.len() < count {
            assert!(Instant::now() < deadline, "timed out waiting for entries");
            let chunk = app.read(id, cursor, MAX_READ_BYTES).unwrap();
            cursor = chunk.next;
            entries.extend(chunk.entries);
            std::thread::sleep(Duration::from_millis(2));
        }
        entries
    }

    #[test]
    fn ports_are_converted() {
        let (app, _) = state();
        let ports = app.ports().unwrap();
        assert_eq!(ports[0].kind, "usb");
        assert_eq!(ports[0].vid, Some(0x10c4));
        assert_eq!(ports[0].pid, Some(0xea60));
        assert_eq!(ports[0].product.as_deref(), Some("CP2102"));
        assert_eq!(ports[0].manufacturer.as_deref(), Some("Silicon Labs"));
        assert_eq!(ports[0].serial_number.as_deref(), Some("0001"));
        let kinds: Vec<&str> = ports.iter().map(|p| p.kind).collect();
        assert_eq!(kinds, ["usb", "bluetooth", "pci", "unknown"]);
        assert_eq!(ports[3].vid, None);
    }

    #[test]
    fn port_listing_errors_become_messages() {
        let app = AppState::new(
            Arc::new(SessionManager::with_serial_ports()),
            Arc::new(|| Err(TransportError::PermissionDenied("udev".into()))),
        );
        assert_eq!(app.ports().unwrap_err(), "permission denied: udev");
    }

    #[test]
    fn open_write_read_close() {
        let (app, devices) = state();
        let session = app.open(&request("mock0")).unwrap();
        assert_eq!(
            session,
            SessionDto {
                id: 1,
                alias: None,
                port: "mock0".into(),
                state: "open",
                error: None,
                auto_reconnect: true,
                ai_access: true,
            }
        );
        assert_eq!(app.write(1, b"AT\r\n").unwrap(), 0);
        let device = devices.lock().unwrap()["mock0"].clone();
        assert_eq!(device.take_written(), b"AT\r\n");
        device.send(b"OK\r\n");

        let entries = read_entries(&app, 1, 2);
        assert_eq!(entries[0].kind, "tx");
        assert_eq!(entries[0].data, b"AT\r\n");
        assert_eq!(entries[1].kind, "rx");
        assert_eq!(entries[1].data, b"OK\r\n");
        assert!(entries[1].timestamp_us >= entries[0].timestamp_us);
        assert_eq!(entries[1].label, None);

        let idle = app.read(1, 2, 1024).unwrap();
        assert!(idle.entries.is_empty());
        assert_eq!(idle.next, 2);
        assert_eq!(idle.state, "open");

        assert_eq!(app.sessions(), std::slice::from_ref(&session));
        assert!(Arc::ptr_eq(app.manager(), &app.manager));
        assert!(app.port_lister()().is_ok());
        app.close(1).unwrap();
        assert!(app.sessions().is_empty());
        assert!(app.read(1, 0, 1024).unwrap_err().contains("no session"));
        assert!(app.close(1).is_err());
        // The port can be opened again after closing.
        assert_eq!(app.open(&request("mock0")).unwrap().id, 2);
        app.shutdown();
    }

    #[test]
    fn markers_and_read_budget() {
        let (app, _) = state();
        app.open(&request("mock0")).unwrap();
        let session = app.manager.get(SessionId(1)).unwrap();
        session.mark("reset");
        app.write(1, b"0123456789").unwrap();
        // A zero budget is clamped to one byte: one entry per read.
        let first = app.read(1, 0, 0).unwrap();
        assert_eq!(first.entries.len(), 1);
        assert_eq!(first.entries[0].kind, "marker");
        assert_eq!(first.entries[0].label.as_deref(), Some("reset"));
        assert!(first.entries[0].data.is_empty());
        assert_eq!(first.next, 1);
        assert_eq!(first.missed, 0);
        assert!(app.read(1, 99, 1).unwrap_err().contains("ahead"));
    }

    #[test]
    fn signals_are_set_and_read() {
        let (app, devices) = state();
        app.open(&request("mock0")).unwrap();
        let device = devices.lock().unwrap()["mock0"].clone();
        device.set_line_states(serialbot_core::transport::LineStates {
            cts: true,
            dsr: true,
            ri: false,
            cd: false,
        });
        let read = app.signals(1).unwrap();
        assert_eq!(
            read,
            SignalsDto {
                id: 1,
                dtr: None,
                rts: None,
                rts_manual: true,
                cts: Some(true),
                dsr: Some(true),
                ri: Some(false),
                cd: Some(false)
            }
        );
        let set = app.set_signals(1, Some(true), None).unwrap();
        assert_eq!((set.dtr, set.rts), (Some(true), None));
        let set = app.set_signals(1, None, Some(false)).unwrap();
        assert_eq!((set.dtr, set.rts), (Some(true), Some(false)));
        assert!(device.dtr() && !device.rts());
        assert!(app.signals(9).unwrap_err().contains("no session"));
        assert!(app.set_signals(9, Some(true), None).is_err());

        device.disconnect();
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.read(1, 0, 1).unwrap().state == "open" {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(app.signals(1).unwrap().cts, None, "unreadable once gone");
        assert!(
            app.set_signals(1, None, Some(true))
                .unwrap_err()
                .contains("Disconnected")
        );
        assert!(
            app.set_signals(1, Some(true), None)
                .unwrap_err()
                .contains("Disconnected")
        );
    }

    #[test]
    fn text_is_encoded_for_sending() {
        let (app, _) = state();
        assert_eq!(
            app.encode_text("温度", "gbk").unwrap(),
            [0xce, 0xc2, 0xb6, 0xc8]
        );
        assert!(
            app.encode_text("x", "nope")
                .unwrap_err()
                .contains("unknown encoding")
        );
    }

    #[test]
    fn repeats_are_started_stopped_and_end_with_the_session() {
        let (app, devices) = state();
        assert!(
            app.start_repeat(1, b"x".to_vec(), 10, None)
                .unwrap_err()
                .contains("no session")
        );
        app.open(&request("mock0")).unwrap();
        let device = devices.lock().unwrap()["mock0"].clone();
        let started = app.start_repeat(1, b"x".to_vec(), 60_000, None).unwrap();
        assert!(started.running);
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.repeat_status(1).sent == 0 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!app.stop_repeat(1).running);
        assert_eq!(device.take_written(), b"x");
        app.start_repeat(1, b"y".to_vec(), 60_000, None).unwrap();
        app.close(1).unwrap();
        assert!(!app.repeat_status(1).running, "closing stops the job");
        app.shutdown();
    }

    #[test]
    fn files_are_sent_stopped_and_end_with_the_session() {
        let (app, devices) = state();
        let dir = crate::settings::tests::TempDir::new("commands-file");
        let path = dir.0.join("fw.bin");
        std::fs::write(&path, b"hello").unwrap();
        let path = path.to_str().unwrap();
        assert!(
            app.send_file(1, path, 2, 0)
                .unwrap_err()
                .contains("no session")
        );
        app.open(&request("mock0")).unwrap();
        let device = devices.lock().unwrap()["mock0"].clone();
        let started = app.send_file(1, path, 2, 0).unwrap();
        assert_eq!((started.running, started.total), (true, 5));
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.file_send_status(1).running {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(device.take_written(), b"hello");
        app.send_file(1, path, 1, 60_000).unwrap();
        let stopped = app.stop_file_send(1);
        assert!(!stopped.running);
        assert!(stopped.sent < 5);
        app.send_file(1, path, 1, 60_000).unwrap();
        app.close(1).unwrap();
        assert_eq!(app.file_send_status(1), FileSendStatus::default());
        app.shutdown();
    }

    #[test]
    fn logs_are_exported_and_captured() {
        let (app, devices) = state();
        let dir = crate::settings::tests::TempDir::new("commands-export");
        let path = dir.0.join("x.bin");
        let path = path.to_str().unwrap();
        assert!(
            app.export_log(1, path, ExportFormat::Raw, 0)
                .unwrap_err()
                .contains("no session")
        );
        assert!(app.start_capture(1, path, ExportFormat::Raw, 0).is_err());
        app.open(&request("mock0")).unwrap();
        devices.lock().unwrap()["mock0"].send(b"hi");
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.read(1, 0, 16).unwrap().entries.is_empty() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            app.export_log(1, path, ExportFormat::Raw, 0).unwrap().bytes,
            2
        );
        let live = dir.0.join("live.bin");
        assert!(
            app.start_capture(1, live.to_str().unwrap(), ExportFormat::Raw, 0)
                .unwrap()
                .active
        );
        while app.capture_status(1).bytes < 2 {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!app.stop_capture(1).active);
        app.close(1).unwrap();
        assert_eq!(
            app.capture_status(1),
            CaptureStatus::default(),
            "closing forgets it"
        );
    }

    #[test]
    fn sessions_can_be_withheld_from_ai_agents() {
        let (app, _devices) = state();
        let opened = app.open(&request("mock0")).unwrap();
        assert!(opened.ai_access);
        app.set_ai_access(1, false).unwrap();
        assert!(!app.sessions()[0].ai_access);
        assert!(app.set_ai_access(9, true).is_err());
        // Entries written from the window are not the AI's.
        app.write(1, b"x").unwrap();
        assert!(!app.read(1, 0, 1024).unwrap().entries[0].ai);
    }

    #[test]
    fn sessions_reconnect_on_request_and_automatically() {
        let (app, devices) = state();
        let manual = OpenRequest {
            auto_reconnect: false,
            ..request("mock0")
        };
        assert!(!app.open(&manual).unwrap().auto_reconnect);
        devices.lock().unwrap()["mock0"].disconnect();
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.read(1, 0, 1).unwrap().state == "open" {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            app.manager().reconnect_lost().is_empty(),
            "auto reconnect is off"
        );
        app.reconnect(1).unwrap();
        assert_eq!(app.read(1, 0, 1).unwrap().state, "open");
        app.set_auto_reconnect(1, true).unwrap();
        assert!(app.sessions()[0].auto_reconnect);
        assert!(app.reconnect(9).unwrap_err().contains("no session"));
        assert!(app.set_auto_reconnect(9, true).is_err());
    }

    #[test]
    fn disconnects_are_reported_by_reads() {
        let (app, devices) = state();
        app.open(&request("mock0")).unwrap();
        devices.lock().unwrap()["mock0"].disconnect();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let chunk = app.read(1, 0, 1024).unwrap();
            if chunk.state == "disconnected" {
                break;
            }
            assert!(Instant::now() < deadline, "disconnect not noticed");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.write(1, b"x").unwrap_err().contains("Disconnected"));
    }

    #[test]
    fn invalid_requests_are_rejected() {
        let (app, _) = state();
        assert!(
            app.open(&request("COM9"))
                .unwrap_err()
                .contains("not found")
        );
        let bad_bits = OpenRequest {
            data_bits: 9,
            ..request("mock0")
        };
        assert!(app.open(&bad_bits).unwrap_err().contains("data bits"));
        let bad_stop = OpenRequest {
            stop_bits: 3,
            ..request("mock0")
        };
        assert!(app.open(&bad_stop).unwrap_err().contains("stop bits"));
        assert!(app.write(1, b"x").unwrap_err().contains("no session"));
        app.open(&request("mock0")).unwrap();
        assert_eq!(app.write(1, b"").unwrap_err(), "nothing to send");
        assert!(
            app.open(&request("mock0"))
                .unwrap_err()
                .contains("already open")
        );
    }

    #[test]
    fn requests_deserialize_with_defaults() {
        let parsed: OpenRequest = serde_json::from_str(r#"{"port":"COM3"}"#).unwrap();
        assert_eq!(parsed, request("COM3"));
        let full: OpenRequest = serde_json::from_str(
            r#"{"port":"COM3","baudRate":9600,"dataBits":7,"parity":"even","stopBits":2,"flowControl":"hardware"}"#,
        )
        .unwrap();
        let config = full.to_config().unwrap();
        assert_eq!(config.baud_rate, 9600);
        assert_eq!(config.data_bits, DataBits::Seven);
        assert_eq!(config.parity, Parity::Even);
        assert_eq!(config.stop_bits, StopBits::Two);
        assert_eq!(config.flow_control, FlowControl::Hardware);
        let odd: OpenRequest =
            serde_json::from_str(r#"{"parity":"odd","flowControl":"software"}"#).unwrap();
        let config = odd.to_config().unwrap();
        assert_eq!(config.parity, Parity::Odd);
        assert_eq!(config.flow_control, FlowControl::Software);
    }

    #[test]
    fn dtos_serialize_in_camel_case() {
        let entry = LogEntryDto {
            seq: 1,
            timestamp_us: 2,
            kind: "rx",
            data: vec![65],
            label: None,
            ai: false,
        };
        assert_eq!(
            serde_json::to_string(&entry).unwrap(),
            r#"{"seq":1,"timestampUs":2,"kind":"rx","data":[65],"label":null,"ai":false}"#
        );
        let failed = state_parts(SessionState::Failed("boom".into()));
        assert_eq!(failed, ("failed", Some("boom".into())));
        assert_eq!(state_parts(SessionState::Closed), ("closed", None));
    }
}
