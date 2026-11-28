// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
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
use crate::sequence::{self, SequenceStatus, Sequences};
use serialbot_jobs::xmodem::Modem;

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
    /// A closed or lost session on the same port to continue (same tab and
    /// log). Without it, the newest such session on the port is continued,
    /// if there is one.
    pub session: Option<u32>,
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
            session: None,
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
    /// `"open"`, `"disconnected"` (device lost), `"closed"` (port closed on
    /// request; the session and log stay until removed) or `"failed"`.
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
    let (state, error) = if summary.released {
        ("closed", None)
    } else {
        state_parts(summary.state.clone())
    };
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

/// One step of a DTR/RTS sequence: set the lines given, then wait.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalStep {
    /// New DTR level, or `None` to leave it.
    pub dtr: Option<bool>,
    /// New RTS level, or `None` to leave it.
    pub rts: Option<bool>,
    /// Pause after setting the lines, in milliseconds.
    #[serde(default)]
    pub wait_ms: u32,
}

/// Most steps of one sequence.
pub const MAX_SEQUENCE_STEPS: usize = 64;
/// Longest total pause of one sequence, in milliseconds.
pub const MAX_SEQUENCE_MS: u64 = 10_000;
/// Longest BREAK, in milliseconds.
pub const MAX_BREAK_MS: u32 = 5_000;

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
    sequences: Sequences,
    decoders: crate::frames::Decoders,
    /// Where the user's protocol files are (`protocols` in the config folder).
    protocols_dir: std::sync::Mutex<Option<std::path::PathBuf>>,
}

/// A protocol's description to edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolSource {
    /// The file name to save it under, without `.toml`.
    pub name: String,
    /// The TOML text.
    pub text: String,
}

/// A protocol the frames window can pick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolDto {
    /// `modbus-rtu`, `slcan`, or `user:<file name>`.
    pub id: String,
    /// Its name (a broken file: the file's name).
    pub name: String,
    /// The protocol file, for the user's own.
    pub file: Option<String>,
    /// What is wrong with the file, naming the line.
    pub error: Option<String>,
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
            sequences: Sequences::default(),
            decoders: crate::frames::Decoders::default(),
            protocols_dir: std::sync::Mutex::new(None),
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

    /// Opens a port. A closed or lost session on the same port (the one
    /// named in the request, or else the newest) continues with the new
    /// settings, keeping its tab and log; otherwise a new session starts.
    pub fn open(&self, request: &OpenRequest) -> Result<SessionDto, String> {
        let config = request.to_config().map_err(|e| e.to_string())?;
        let resumable = self
            .manager
            .list()
            .into_iter()
            .filter(|s| s.state != SessionState::Open && s.state != SessionState::Closed)
            .filter(|s| s.info.name == config.path)
            .map(|s| s.id.0)
            .filter(|id| request.session.is_none_or(|wanted| wanted == *id))
            .max();
        let id = match resumable {
            Some(id) => {
                self.manager
                    .reopen(SessionId(id), &config)
                    .map_err(|e| e.to_string())?;
                SessionId(id)
            }
            None => self
                .manager
                .open(&config, None)
                .map_err(|e| e.to_string())?
                .id(),
        };
        self.manager
            .set_auto_reconnect(id, request.auto_reconnect)
            .map_err(|e| e.to_string())?;
        self.manager
            .list()
            .iter()
            .find(|s| s.id == id)
            .map(session_dto)
            .ok_or_else(|| "the session closed while opening".to_owned())
    }

    /// Closes a session's port but keeps the session and its log (its tab
    /// stays). Timed and file sends and test sequences stop; a capture to file keeps its file
    /// and continues after a reopen.
    pub fn release(&self, id: u32) -> Result<(), String> {
        self.repeaters.stop(id);
        self.file_sends.stop(id);
        self.sequences.stop(id);
        self.manager
            .release(SessionId(id))
            .map_err(|e| e.to_string())
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

    /// Closes a session and forgets it with its log (closing its tab).
    pub fn close(&self, id: u32) -> Result<(), String> {
        self.repeaters.stop(id);
        self.file_sends.remove(id);
        self.captures.remove(id);
        self.sequences.remove(id);
        self.decoders.remove(id);
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
        let (state, error) = if self.manager.is_released(SessionId(id)) {
            ("closed", None)
        } else {
            state_parts(session.state())
        };
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

    /// Runs a DTR/RTS sequence (for example a reset or boot-mode pulse)
    /// with the pauses timed here rather than across IPC calls. Every
    /// change is logged like [`AppState::set_signals`]. Refuses more than
    /// [`MAX_SEQUENCE_STEPS`] steps or more than [`MAX_SEQUENCE_MS`] of
    /// pauses before touching a line; stops at the first failing step.
    pub fn run_signals(&self, id: u32, steps: &[SignalStep]) -> Result<SignalsDto, String> {
        if steps.len() > MAX_SEQUENCE_STEPS {
            return Err(format!("a sequence has at most {MAX_SEQUENCE_STEPS} steps"));
        }
        let total: u64 = steps.iter().map(|s| u64::from(s.wait_ms)).sum();
        if total > MAX_SEQUENCE_MS {
            return Err(format!(
                "a sequence pauses at most {MAX_SEQUENCE_MS} ms in all"
            ));
        }
        let session = self.manager.get(SessionId(id)).map_err(|e| e.to_string())?;
        for step in steps {
            if let Some(level) = step.dtr {
                session.set_dtr(level).map_err(|e| e.to_string())?;
            }
            if let Some(level) = step.rts {
                session.set_rts(level).map_err(|e| e.to_string())?;
            }
            if step.wait_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(u64::from(step.wait_ms)));
            }
        }
        Ok(signals_dto(&session))
    }

    /// Holds a BREAK for `ms` milliseconds (1 to [`MAX_BREAK_MS`]); it is
    /// logged as a marker.
    pub fn send_break(&self, id: u32, ms: u32) -> Result<(), String> {
        if !(1..=MAX_BREAK_MS).contains(&ms) {
            return Err(format!("a BREAK lasts 1 to {MAX_BREAK_MS} ms"));
        }
        let session = self.manager.get(SessionId(id)).map_err(|e| e.to_string())?;
        session
            .send_break(std::time::Duration::from_millis(u64::from(ms)))
            .map_err(|e| e.to_string())
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
        self.sequences.stop_all();
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
            &session,
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

    /// Starts running the test sequence `script` (see
    /// [`serialbot_jobs::script`]) on the session in the background,
    /// replacing its previous one. Sends get `ending` (`none`, `cr`, `lf`,
    /// `crlf`) until an `ending` line changes it and are encoded in
    /// `encoding`. `runs` `None` runs until stopped; `stop_on_fail` ends it
    /// at the first failed run; `gap_ms` is the pause between runs. Fails
    /// for a script error (naming its line) or bad options.
    #[allow(clippy::too_many_arguments, reason = "one IPC call's fields")]
    pub fn start_sequence(
        &self,
        id: u32,
        name: &str,
        script: &str,
        ending: &str,
        encoding: &str,
        runs: Option<u32>,
        stop_on_fail: bool,
        gap_ms: u64,
    ) -> Result<SequenceStatus, String> {
        use serialbot_jobs::script::{Ending, parse_text};
        let ending = match ending {
            "none" => Ending::None,
            "cr" => Ending::Cr,
            "lf" => Ending::Lf,
            "crlf" => Ending::Crlf,
            other => return Err(format!("unknown line ending {other:?}")),
        };
        let items = parse_text(script, ending, encoding)?;
        let session = self.manager.get(SessionId(id)).map_err(|e| e.to_string())?;
        let options = sequence::Options {
            runs,
            stop_on_fail,
            gap: std::time::Duration::from_millis(gap_ms),
        };
        self.sequences
            .start(id, session, name.to_owned(), items, options)?;
        Ok(self.sequences.status(id))
    }

    /// Sets the folder of the user's protocol files.
    pub fn set_protocols_dir(&self, dir: std::path::PathBuf) {
        *self
            .protocols_dir
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(dir);
    }

    fn protocols_dir(&self) -> Option<std::path::PathBuf> {
        self.protocols_dir
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The built-in protocols, then every protocol file in the folder (a
    /// broken one with its mistake, so the window can say what to fix).
    pub fn protocols(&self) -> Vec<ProtocolDto> {
        let mut list: Vec<ProtocolDto> = serialbot_decode::PROTOCOLS
            .iter()
            .map(|p| ProtocolDto {
                id: p.id.to_owned(),
                name: p.name.to_owned(),
                file: None,
                error: None,
            })
            .collect();
        if let Some(dir) = self.protocols_dir() {
            list.extend(
                serialbot_decode::custom::load_dir(&dir)
                    .into_iter()
                    .map(|p| {
                        let file_name = p
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        let (name, error) = match p.protocol {
                            Ok(c) => (c.name().to_owned(), None),
                            Err(e) => (file_name, Some(e)),
                        };
                        ProtocolDto {
                            id: p.id,
                            name,
                            file: Some(p.path.display().to_string()),
                            error,
                        }
                    }),
            );
        }
        list
    }

    /// The protocols folder, made (with an example file to copy) if it is
    /// not there yet.
    pub fn ensure_protocols_dir(&self) -> Result<std::path::PathBuf, String> {
        let dir = self
            .protocols_dir()
            .ok_or("this system has no configuration folder")?;
        if !dir.exists() {
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            std::fs::write(dir.join("example.toml"), serialbot_decode::custom::EXAMPLE)
                .map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        Ok(dir)
    }

    /// The text of protocol `id` to edit, with the file name (without
    /// `.toml`) to save it under: a file of the user's as it is, anything
    /// else as the example to start from (`my-protocol`).
    pub fn protocol_source(&self, id: &str) -> Result<ProtocolSource, String> {
        if let Some(stem) = id.strip_prefix("user:") {
            let dir = self
                .protocols_dir()
                .ok_or("this system has no configuration folder")?;
            let path = dir.join(format!("{stem}.toml"));
            let text =
                std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            return Ok(ProtocolSource {
                name: stem.to_owned(),
                text,
            });
        }
        Ok(ProtocolSource {
            name: "my-protocol".to_owned(),
            text: serialbot_decode::custom::EXAMPLE.to_owned(),
        })
    }

    /// The frames the description `text` makes of `hex` (bytes as hex, taken
    /// as received), named in `lang`; or the description's mistake (with its
    /// line), or why `hex` is not hex. Nothing is saved.
    pub fn preview_protocol(
        &self,
        text: &str,
        hex: &str,
        lang: &str,
    ) -> Result<Vec<serialbot_decode::Frame>, String> {
        let custom = serialbot_decode::custom::parse(text)?;
        let bytes = if hex.trim().is_empty() {
            Vec::new()
        } else {
            serialbot_core::hex::parse_hex(hex)?
        };
        let lang = if lang == "zh" {
            serialbot_decode::Lang::Zh
        } else {
            serialbot_decode::Lang::En
        };
        let mut decoder = custom.decoder(lang);
        Ok(serialbot_decode::decode_all(
            decoder.as_mut(),
            [(serialbot_decode::Direction::Rx, 0, bytes.as_slice())],
        ))
    }

    /// Saves `text` as `<name>.toml` in the protocols folder (made if
    /// missing), replacing a file of that name, and gives its id
    /// (`user:<name>`). A name is 1 to 64 letters, digits, `-`, `_` or `.`,
    /// not starting with `.`. The text is saved even with mistakes (the
    /// list shows them), so work in progress is kept.
    pub fn save_protocol(&self, name: &str, text: &str) -> Result<String, String> {
        let ok = !name.is_empty()
            && name.len() <= 64
            && !name.starts_with('.')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if !ok {
            return Err(format!(
                "{name:?} cannot be a file name: use letters, digits, - _ and ."
            ));
        }
        let dir = self.ensure_protocols_dir()?;
        let path = dir.join(format!("{name}.toml"));
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(format!("user:{name}"))
    }

    /// Up to `max` (at most 2000) frames of the session's log decoded as
    /// `protocol` (a built-in id or `user:<file name>`), from number
    /// `after` on, named in `lang` (`zh` or anything else for English); see
    /// [`crate::frames::Decoders::read`]. A protocol file is read each time,
    /// so a saved change shows at once.
    pub fn read_frames(
        &self,
        id: u32,
        protocol: &str,
        lang: &str,
        after: u64,
        max: usize,
    ) -> Result<crate::frames::FramesChunk, String> {
        use std::hash::{Hash, Hasher};
        let session = self.manager.get(SessionId(id)).map_err(|e| e.to_string())?;
        let lang = if lang == "zh" {
            serialbot_decode::Lang::Zh
        } else {
            serialbot_decode::Lang::En
        };
        let dir = self.protocols_dir();
        let key = match protocol.strip_prefix("user:") {
            Some(stem) => {
                let path = dir
                    .as_ref()
                    .map(|d| d.join(format!("{stem}.toml")))
                    .ok_or("this system has no configuration folder")?;
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                text.hash(&mut hash);
                format!("{protocol}#{:x}", hash.finish())
            }
            None => protocol.to_owned(),
        };
        let make =
            || serialbot_decode::find_decoder(protocol, dir.as_deref(), lang).map(|(_, d)| d);
        self.decoders
            .read(id, &session, &key, lang, &make, after, max.clamp(1, 2000))
    }

    /// Stops the session's test sequence; its tally stays readable.
    pub fn stop_sequence(&self, id: u32) -> SequenceStatus {
        self.sequences.stop(id);
        self.sequences.status(id)
    }

    /// Where the session's test sequence is, and its tally so far.
    pub fn sequence_status(&self, id: u32) -> SequenceStatus {
        self.sequences.status(id)
    }

    /// Starts sending the file at `path`, replacing the session's previous
    /// file job. With `protocol` `None` or `raw` it is written in chunks of
    /// `chunk` bytes with `delay_ms` after each (see [`FileSends::start`]
    /// for the limits); with `xmodem`, `xmodem-1k`, `ymodem` or
    /// `ymodem-128` it goes to a bootloader, which must start the transfer
    /// within a minute (see [`FileSends::start_modem`]).
    pub fn send_file(
        &self,
        id: u32,
        path: &str,
        chunk: usize,
        delay_ms: u64,
        protocol: Option<&str>,
    ) -> Result<FileSendStatus, String> {
        let session = self.manager.get(SessionId(id)).map_err(|e| e.to_string())?;
        let path = std::path::Path::new(path);
        match protocol.filter(|p| !p.eq_ignore_ascii_case("raw")) {
            None => self.file_sends.start(id, session, path, chunk, delay_ms),
            Some(name) => {
                let modem = Modem::parse(name)
                    .ok_or_else(|| format!("unknown file transfer protocol {name}"))?;
                self.file_sends.start_modem(id, session, path, modem)
            }
        }
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

        // A reset pulse: both lines set, a pause, then DTR back.
        let start = std::time::Instant::now();
        let steps = [
            SignalStep {
                dtr: Some(false),
                rts: Some(true),
                wait_ms: 30,
            },
            SignalStep {
                rts: Some(false),
                ..SignalStep::default()
            },
        ];
        let after = app.run_signals(1, &steps).unwrap();
        assert!(start.elapsed() >= std::time::Duration::from_millis(30));
        assert_eq!((after.dtr, after.rts), (Some(false), Some(false)));
        let too_many = vec![SignalStep::default(); MAX_SEQUENCE_STEPS + 1];
        assert!(app.run_signals(1, &too_many).unwrap_err().contains("steps"));
        let too_long = [SignalStep {
            wait_ms: 10_001,
            ..SignalStep::default()
        }];
        assert!(app.run_signals(1, &too_long).unwrap_err().contains("ms"));
        assert!(
            app.run_signals(9, &steps)
                .unwrap_err()
                .contains("no session")
        );
        let parsed: SignalStep = serde_json::from_str(r#"{"dtr":true,"rts":null}"#).unwrap();
        assert_eq!(
            parsed,
            SignalStep {
                dtr: Some(true),
                rts: None,
                wait_ms: 0
            }
        );

        app.send_break(1, 20).unwrap();
        assert_eq!(device.breaks(), [std::time::Duration::from_millis(20)]);
        assert!(app.send_break(1, 0).unwrap_err().contains("1 to 5000"));
        assert!(app.send_break(1, 5001).is_err());
        assert!(app.send_break(9, 20).unwrap_err().contains("no session"));
        let labels: Vec<String> = app
            .read(1, 0, 4096)
            .unwrap()
            .entries
            .into_iter()
            .filter_map(|e| e.label)
            .collect();
        assert_eq!(
            labels,
            ["DTR=1", "RTS=0", "DTR=0", "RTS=1", "RTS=0", "BREAK 20 ms"]
        );
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
            app.send_file(1, path, 2, 0, None)
                .unwrap_err()
                .contains("no session")
        );
        app.open(&request("mock0")).unwrap();
        let device = devices.lock().unwrap()["mock0"].clone();
        let started = app.send_file(1, path, 2, 0, None).unwrap();
        assert_eq!((started.running, started.total), (true, 5));
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.file_send_status(1).running {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(device.take_written(), b"hello");
        app.send_file(1, path, 1, 60_000, Some("raw")).unwrap();
        let stopped = app.stop_file_send(1);
        assert!(!stopped.running);
        assert!(stopped.sent < 5);
        assert!(
            app.send_file(1, path, 1, 0, Some("zmodem"))
                .unwrap_err()
                .contains("unknown file transfer protocol")
        );
        let modem = app.send_file(1, path, 1, 0, Some("ymodem")).unwrap();
        assert_eq!(modem.protocol.as_deref(), Some("YMODEM"));
        assert!(app.file_send_status(1).running, "waits for the receiver");
        app.close(1).unwrap();
        assert_eq!(app.file_send_status(1), FileSendStatus::default());
        app.shutdown();
    }

    #[test]
    fn test_sequences_run_stop_and_end_with_the_session() {
        let (app, devices) = state();
        let start = |script: &str, ending: &str, runs| {
            app.start_sequence(1, "check", script, ending, "utf-8", runs, true, 0)
        };
        assert!(start("send x", "crlf", Some(1)).is_err(), "no session yet");
        app.open(&request("mock0")).unwrap();
        assert!(
            start("frobnicate", "crlf", Some(1))
                .unwrap_err()
                .contains("line 1")
        );
        assert!(start("send x", "dos", Some(1)).is_err());
        assert!(start("send x", "crlf", Some(0)).is_err());

        let started = start("send x\nwait 1ms", "lf", Some(2)).unwrap();
        assert_eq!(started.name, "check");
        wait_until(|| !app.sequence_status(1).running);
        let done = app.sequence_status(1);
        assert_eq!((done.done, done.passed, done.failed), (2, 2, 0));
        assert_eq!(devices.lock().unwrap()["mock0"].take_written(), b"x\nx\n");

        for ending in ["none", "cr", "crlf"] {
            start("wait 1ms", ending, Some(1)).unwrap();
        }
        start("wait 1s", "none", None).unwrap();
        assert!(app.sequence_status(1).running);
        let stopped = app.stop_sequence(1);
        assert!(stopped.stopped && !stopped.running, "{stopped:?}");

        start("wait 1s", "none", None).unwrap();
        app.release(1).unwrap();
        assert!(!app.sequence_status(1).running, "closing the port stops it");
        app.close(1).unwrap();
        assert_eq!(app.sequence_status(1), SequenceStatus::default());
        app.shutdown();
    }

    #[test]
    fn frames_are_read_by_protocol_and_forgotten_with_the_session() {
        let (app, devices) = state();
        assert!(
            app.read_frames(1, "slcan", "en", 0, 10).is_err(),
            "no session"
        );
        app.open(&request("mock0")).unwrap();
        devices.lock().unwrap()["mock0"].send(b"t1230\r");
        wait_until(|| !app.read(1, 0, 16).unwrap().entries.is_empty());
        let en = app.read_frames(1, "slcan", "en", 0, 0).unwrap();
        assert_eq!(en.frames.len(), 1, "at least one frame per read");
        assert_eq!(en.frames[0].frame.name, "CAN standard frame");
        let zh = app.read_frames(1, "slcan", "zh", 0, 10).unwrap();
        assert_eq!(zh.frames[0].frame.name, "CAN 标准帧");
        assert!(app.read_frames(1, "x", "en", 0, 10).is_err());
        assert!(
            app.read_frames(1, "user:x", "en", 0, 10).is_err(),
            "no folder"
        );

        // The user's own protocols, from the folder.
        assert_eq!(app.protocols().len(), 2);
        assert!(app.ensure_protocols_dir().is_err());
        let dir = crate::settings::tests::TempDir::new("protocols");
        let folder = dir.0.join("protocols");
        app.set_protocols_dir(folder.clone());
        assert_eq!(app.ensure_protocols_dir().unwrap(), folder);
        std::fs::write(
            folder.join("example.toml"),
            "name = \"Mine\"\n[frame]\nend = \"0D\"\n",
        )
        .unwrap();
        assert_eq!(app.ensure_protocols_dir().unwrap(), folder, "kept");
        std::fs::write(folder.join("broken.toml"), "nope").unwrap();
        let list = app.protocols();
        let ids: Vec<&str> = list.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["modbus-rtu", "slcan", "user:broken", "user:example"]);
        assert_eq!(list[2].name, "broken.toml");
        assert!(list[2].error.is_some());
        assert_eq!(list[3].name, "Mine");
        let mine = app.read_frames(1, "user:example", "en", 0, 10).unwrap();
        assert_eq!(mine.frames[0].frame.name, "Mine");
        // A saved change decodes afresh.
        std::fs::write(
            folder.join("example.toml"),
            "name = \"Yours\"\n[frame]\nend = \"0D\"\n",
        )
        .unwrap();
        let yours = app.read_frames(1, "user:example", "en", 0, 10).unwrap();
        assert_eq!(yours.frames[0].frame.name, "Yours");
        assert!(
            app.read_frames(1, "user:broken", "en", 0, 10)
                .unwrap_err()
                .contains("broken.toml")
        );
        app.close(1).unwrap();
        assert!(app.read_frames(1, "slcan", "en", 0, 10).is_err());
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

    /// Waits up to five seconds for `done`.
    fn wait_until(mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !done() {
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn repeats_with_a_count_end_by_themselves() {
        let (app, devices) = state();
        app.open(&request("mock0")).unwrap();
        app.start_repeat(1, b"x".to_vec(), 1, Some(3)).unwrap();
        wait_until(|| !app.repeat_status(1).running);
        let status = app.repeat_status(1);
        assert_eq!((status.sent, status.error), (3, None));
        assert_eq!(devices.lock().unwrap()["mock0"].take_written(), b"xxx");
        assert!(
            app.start_repeat(1, Vec::new(), 10, None).is_err(),
            "nothing to send"
        );
        app.shutdown();
    }

    #[test]
    fn closing_the_port_stops_sends_and_keeps_the_capture() {
        let (app, devices) = state();
        let dir = crate::settings::tests::TempDir::new("commands-release");
        let fw = dir.0.join("fw.bin");
        std::fs::write(&fw, b"0123456789").unwrap();
        let live = dir.0.join("live.log");
        app.open(&request("mock0")).unwrap();
        app.start_repeat(1, b"r".to_vec(), 60_000, None).unwrap();
        app.send_file(1, fw.to_str().unwrap(), 1, 60_000, None)
            .unwrap();
        app.start_capture(1, live.to_str().unwrap(), ExportFormat::Raw, 0)
            .unwrap();
        devices.lock().unwrap()["mock0"].send(b"one ");
        wait_until(|| app.capture_status(1).bytes >= 4);

        app.release(1).unwrap();
        // Long enough for the capture to notice the closed port.
        std::thread::sleep(Duration::from_millis(400));
        assert!(!app.repeat_status(1).running, "timed send stops");
        assert!(!app.file_send_status(1).running, "file send stops");
        assert!(app.capture_status(1).active, "the capture keeps its file");

        // Reopening continues the session, and the capture with it.
        app.open(&request("mock0")).unwrap();
        devices.lock().unwrap()["mock0"].send(b"two");
        wait_until(|| app.capture_status(1).bytes >= 7);
        app.stop_capture(1);
        assert_eq!(std::fs::read(&live).unwrap(), b"one two");
        app.shutdown();
    }

    #[test]
    fn log_exports_show_time_and_direction() {
        let (app, devices) = state();
        let dir = crate::settings::tests::TempDir::new("commands-log");
        let path = dir.0.join("session.log");
        let path = path.to_str().unwrap();
        app.open(&request("mock0")).unwrap();
        app.write(1, b"AT\r\n").unwrap();
        devices.lock().unwrap()["mock0"].send(b"OK\r\n");
        wait_until(|| {
            app.read(1, 0, 16)
                .unwrap()
                .entries
                .iter()
                .any(|e| e.data == b"OK\r\n")
        });
        app.export_log(1, path, ExportFormat::Log, 0).unwrap();
        let text = std::fs::read_to_string(path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines.iter().any(|l| l.ends_with(" TX AT\\r\\n")), "{text}");
        assert!(lines.iter().any(|l| l.ends_with(" RX OK\\r\\n")), "{text}");
        // Each line starts with a local date and time.
        assert!(
            lines.iter().all(|l| l.as_bytes().get(4) == Some(&b'-')),
            "{text}"
        );
        app.shutdown();
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
    fn released_sessions_keep_their_log_and_continue_on_open() {
        let (app, devices) = state();
        app.open(&request("mock0")).unwrap();
        app.write(1, b"before").unwrap();
        app.release(1).unwrap();
        // Still listed, as closed, with its log.
        assert_eq!(app.sessions()[0].state, "closed");
        let chunk = app.read(1, 0, 1024).unwrap();
        assert_eq!(chunk.state, "closed");
        assert!(chunk.entries.iter().any(|e| e.data == b"before"));
        assert!(app.write(1, b"x").is_err(), "the port is closed");
        // Opening the port again continues the same session.
        let reopened = app
            .open(&OpenRequest {
                baud_rate: 9600,
                ..request("mock0")
            })
            .unwrap();
        assert_eq!(reopened.id, 1);
        assert_eq!(reopened.state, "open");
        assert_eq!(app.sessions().len(), 1);
        app.write(1, b"after").unwrap();
        assert_eq!(devices.lock().unwrap()["mock0"].take_written(), b"after");
        // Releasing twice is fine; the tab's × removes it.
        app.release(1).unwrap();
        app.release(1).unwrap();
        app.close(1).unwrap();
        assert!(app.sessions().is_empty());
        assert!(app.release(1).is_err());
    }

    #[test]
    fn open_continues_the_named_session_or_the_newest() {
        let (app, _) = state();
        app.open(&request("mock0")).unwrap();
        app.release(1).unwrap();
        // With session 1 closed, a new open starts session 1 again (newest).
        assert_eq!(app.open(&request("mock0")).unwrap().id, 1);
        // Two closed sessions on the same port: the named one continues.
        app.release(1).unwrap();
        let other = app.open(&request("mock1")).unwrap().id;
        assert_ne!(other, 1);
        let named = OpenRequest {
            session: Some(1),
            ..request("mock0")
        };
        assert_eq!(app.open(&named).unwrap().id, 1);
        // An open session elsewhere is not continued: a named session on
        // another port is ignored and a new one starts.
        let elsewhere = OpenRequest {
            session: Some(other),
            ..request("mock2")
        };
        let fresh = app.open(&elsewhere).unwrap().id;
        assert!(fresh != 1 && fresh != other);
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
