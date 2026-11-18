//! SerialBot desktop app: a Tauri shell whose window is one more view of the
//! sessions owned by the core's [`serialbot_core::SessionManager`]. The
//! embedded MCP server ([`mcp`]) is another view of the same sessions.
//!
//! The IPC commands below only forward to tested modules. They run off the
//! main thread (`async`) so that opening a slow port never freezes the
//! window. Changes reach the window as events (see [`watch`]):
//! `sessions-changed` carries the session list, `log-changed` the ids of
//! sessions with new log entries, `signals-changed` the line levels of one
//! session, `ports-changed` the port list after a device is plugged in or
//! removed. The same thread retries lost sessions that have auto-reconnect
//! on.

pub mod commands;
pub mod export;
pub mod filesend;
pub mod mcp;
pub mod repeat;
pub mod settings;
pub mod watch;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use commands::AppState;
use mcp::McpController;
use tauri::{Emitter, Manager};

/// How often the change detector runs (about 60 times a second).
const WATCH_INTERVAL: Duration = Duration::from_millis(16);

/// Line levels are read every this many watch ticks (about 10 times a
/// second), because reading them may cost a USB round trip.
const SIGNAL_TICKS: u32 = 6;

/// Lost sessions with auto-reconnect are retried about once a second.
const RECONNECT_TICKS: u32 = 60;

/// The port list is checked for plugged/unplugged devices about every two
/// seconds.
const PORT_TICKS: u32 = 120;

/// Thin IPC wrappers. Tauri hands command arguments over by value, and
/// async commands that borrow state must return `Result`.
#[allow(
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    reason = "signatures required by tauri::command"
)]
mod ipc {
    use super::commands::{AppState, LogChunk, OpenRequest, PortDto, SessionDto, SignalsDto};
    use super::export::{CaptureStatus, ExportFormat, ExportSummary};
    use super::filesend::FileSendStatus;
    use super::mcp::{McpController, McpStatus};
    use super::repeat::RepeatStatus;
    use super::settings::McpSettings;

    #[tauri::command(async)]
    pub(crate) fn list_ports(state: tauri::State<'_, AppState>) -> Result<Vec<PortDto>, String> {
        state.ports()
    }

    #[tauri::command(async)]
    pub(crate) fn list_sessions(
        state: tauri::State<'_, AppState>,
    ) -> Result<Vec<SessionDto>, String> {
        Ok(state.sessions())
    }

    #[tauri::command(async)]
    pub(crate) fn open_session(
        state: tauri::State<'_, AppState>,
        request: OpenRequest,
    ) -> Result<SessionDto, String> {
        state.open(&request)
    }

    #[tauri::command(async)]
    pub(crate) fn close_session(state: tauri::State<'_, AppState>, id: u32) -> Result<(), String> {
        state.close(id)
    }

    #[tauri::command(async)]
    pub(crate) fn write_data(
        state: tauri::State<'_, AppState>,
        id: u32,
        data: Vec<u8>,
    ) -> Result<u64, String> {
        state.write(id, &data)
    }

    #[tauri::command(async)]
    pub(crate) fn read_log(
        state: tauri::State<'_, AppState>,
        id: u32,
        cursor: u64,
        max_bytes: usize,
    ) -> Result<LogChunk, String> {
        state.read(id, cursor, max_bytes)
    }

    #[tauri::command(async)]
    pub(crate) fn reconnect(state: tauri::State<'_, AppState>, id: u32) -> Result<(), String> {
        state.reconnect(id)
    }

    #[tauri::command(async)]
    pub(crate) fn set_auto_reconnect(
        state: tauri::State<'_, AppState>,
        id: u32,
        on: bool,
    ) -> Result<(), String> {
        state.set_auto_reconnect(id, on)
    }

    #[tauri::command(async)]
    pub(crate) fn export_log(
        state: tauri::State<'_, AppState>,
        id: u32,
        path: String,
        format: ExportFormat,
        utc_offset_min: i32,
    ) -> Result<ExportSummary, String> {
        state.export_log(id, &path, format, utc_offset_min)
    }

    #[tauri::command(async)]
    pub(crate) fn start_capture(
        state: tauri::State<'_, AppState>,
        id: u32,
        path: String,
        format: ExportFormat,
        utc_offset_min: i32,
    ) -> Result<CaptureStatus, String> {
        state.start_capture(id, &path, format, utc_offset_min)
    }

    #[tauri::command(async)]
    pub(crate) fn stop_capture(
        state: tauri::State<'_, AppState>,
        id: u32,
    ) -> Result<CaptureStatus, String> {
        Ok(state.stop_capture(id))
    }

    #[tauri::command(async)]
    pub(crate) fn capture_status(
        state: tauri::State<'_, AppState>,
        id: u32,
    ) -> Result<CaptureStatus, String> {
        Ok(state.capture_status(id))
    }

    #[tauri::command(async)]
    pub(crate) fn start_repeat(
        state: tauri::State<'_, AppState>,
        id: u32,
        data: Vec<u8>,
        interval_ms: u64,
        count: Option<u32>,
    ) -> Result<RepeatStatus, String> {
        state.start_repeat(id, data, interval_ms, count)
    }

    #[tauri::command(async)]
    pub(crate) fn stop_repeat(
        state: tauri::State<'_, AppState>,
        id: u32,
    ) -> Result<RepeatStatus, String> {
        Ok(state.stop_repeat(id))
    }

    #[tauri::command(async)]
    pub(crate) fn repeat_status(
        state: tauri::State<'_, AppState>,
        id: u32,
    ) -> Result<RepeatStatus, String> {
        Ok(state.repeat_status(id))
    }

    #[tauri::command(async)]
    pub(crate) fn send_file(
        state: tauri::State<'_, AppState>,
        id: u32,
        path: String,
        chunk: usize,
        delay_ms: u64,
    ) -> Result<FileSendStatus, String> {
        state.send_file(id, &path, chunk, delay_ms)
    }

    #[tauri::command(async)]
    pub(crate) fn stop_file_send(
        state: tauri::State<'_, AppState>,
        id: u32,
    ) -> Result<FileSendStatus, String> {
        Ok(state.stop_file_send(id))
    }

    #[tauri::command(async)]
    pub(crate) fn file_send_status(
        state: tauri::State<'_, AppState>,
        id: u32,
    ) -> Result<FileSendStatus, String> {
        Ok(state.file_send_status(id))
    }

    #[tauri::command(async)]
    pub(crate) fn encode_text(
        state: tauri::State<'_, AppState>,
        text: String,
        encoding: String,
    ) -> Result<Vec<u8>, String> {
        state.encode_text(&text, &encoding)
    }

    #[tauri::command(async)]
    pub(crate) fn signals(
        state: tauri::State<'_, AppState>,
        id: u32,
    ) -> Result<SignalsDto, String> {
        state.signals(id)
    }

    #[tauri::command(async)]
    pub(crate) fn set_signals(
        state: tauri::State<'_, AppState>,
        id: u32,
        dtr: Option<bool>,
        rts: Option<bool>,
    ) -> Result<SignalsDto, String> {
        state.set_signals(id, dtr, rts)
    }

    #[tauri::command]
    pub(crate) async fn mcp_status(
        mcp: tauri::State<'_, McpController>,
    ) -> Result<McpStatus, String> {
        Ok(mcp.status().await)
    }

    #[tauri::command]
    pub(crate) async fn mcp_apply(
        mcp: tauri::State<'_, McpController>,
        settings: McpSettings,
    ) -> Result<McpStatus, String> {
        mcp.apply(settings).await
    }

    #[tauri::command]
    pub(crate) async fn mcp_regenerate_token(
        mcp: tauri::State<'_, McpController>,
    ) -> Result<McpStatus, String> {
        mcp.regenerate_token().await
    }
}

/// Background thread that turns detected changes into window events.
struct Watcher {
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Watcher {
    fn start(
        handle: tauri::AppHandle,
        manager: Arc<serialbot_core::SessionManager>,
        lister: Arc<commands::PortLister>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let mut detector = watch::ChangeDetector::default();
            let mut tick = 0u32;
            while !flag.load(Ordering::Relaxed) {
                let changes = detector.poll(&manager);
                // Emitting fails only while the window is gone; the window
                // reloads the full state when it comes back.
                if let Some(sessions) = changes.sessions {
                    let _ = handle.emit("sessions-changed", sessions);
                }
                if !changes.logs.is_empty() {
                    let _ = handle.emit("log-changed", changes.logs);
                }
                if tick.is_multiple_of(RECONNECT_TICKS) {
                    // Reconnected sessions show up as state changes.
                    manager.reconnect_lost();
                }
                if tick.is_multiple_of(PORT_TICKS)
                    && let Some(ports) = detector.poll_ports(&*lister)
                {
                    let _ = handle.emit("ports-changed", ports);
                }
                if tick.is_multiple_of(SIGNAL_TICKS) {
                    for signals in detector.poll_signals(&manager) {
                        let _ = handle.emit("signals-changed", signals);
                    }
                }
                tick = tick.wrapping_add(1);
                std::thread::sleep(WATCH_INTERVAL);
            }
        });
        Self {
            stop,
            thread: Mutex::new(Some(thread)),
        }
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        let thread = self
            .thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(thread) = thread {
            // A panicked watcher has nothing to clean up.
            let _ = thread.join();
        }
    }
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let state = app.state::<AppState>();
    let manager = Arc::clone(state.manager());
    let lister = Arc::clone(state.port_lister());
    let path = app.path().app_config_dir()?.join("mcp.json");
    let (settings, note) = match settings::load(&path) {
        Ok(loaded) => loaded,
        Err(e) => (settings::McpSettings::new_default()?, Some(e)),
    };
    app.manage(McpController::new(
        Arc::clone(&manager),
        Arc::clone(&lister),
        settings,
        Some(path),
        note,
    ));
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        handle.state::<McpController>().start_if_enabled().await;
    });
    app.manage(Watcher::start(app.handle().clone(), manager, lister));
    Ok(())
}

/// Starts the app and blocks until its last window closes; the MCP server
/// and all ports are closed on exit.
pub fn run() -> Result<(), tauri::Error> {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::with_serial_ports())
        .setup(setup)
        .invoke_handler(tauri::generate_handler![
            ipc::list_ports,
            ipc::list_sessions,
            ipc::open_session,
            ipc::close_session,
            ipc::write_data,
            ipc::read_log,
            ipc::send_file,
            ipc::stop_file_send,
            ipc::file_send_status,
            ipc::encode_text,
            ipc::reconnect,
            ipc::set_auto_reconnect,
            ipc::export_log,
            ipc::start_capture,
            ipc::stop_capture,
            ipc::capture_status,
            ipc::start_repeat,
            ipc::stop_repeat,
            ipc::repeat_status,
            ipc::signals,
            ipc::set_signals,
            ipc::mcp_status,
            ipc::mcp_apply,
            ipc::mcp_regenerate_token
        ])
        .build(tauri::generate_context!())?;
    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            handle.state::<Watcher>().stop();
            tauri::async_runtime::block_on(handle.state::<McpController>().stop());
            handle.state::<AppState>().shutdown();
        }
    });
    Ok(())
}
