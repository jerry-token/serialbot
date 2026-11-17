//! serialx desktop app: a Tauri shell whose window is one more view of the
//! sessions owned by the core's [`serialx_core::SessionManager`].
//!
//! The commands below only forward to [`commands::AppState`], which holds
//! the tested logic. They run off the main thread (`async`) so that opening
//! a slow port never freezes the window.

pub mod commands;

use commands::AppState;
use tauri::Manager;

/// Thin IPC wrappers; Tauri hands command arguments over by value.
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri passes command arguments by value"
)]
mod ipc {
    use super::commands::{AppState, LogChunk, OpenRequest, PortDto, SessionDto};

    #[tauri::command(async)]
    pub(crate) fn list_ports(state: tauri::State<'_, AppState>) -> Result<Vec<PortDto>, String> {
        state.ports()
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
}

/// Starts the app and blocks until its last window closes; all ports are
/// closed on exit.
pub fn run() -> Result<(), tauri::Error> {
    let app = tauri::Builder::default()
        .manage(AppState::with_serial_ports())
        .invoke_handler(tauri::generate_handler![
            ipc::list_ports,
            ipc::open_session,
            ipc::close_session,
            ipc::write_data,
            ipc::read_log
        ])
        .build(tauri::generate_context!())?;
    app.run(|handle, event| {
        if let tauri::RunEvent::Exit = event {
            handle.state::<AppState>().shutdown();
        }
    });
    Ok(())
}
