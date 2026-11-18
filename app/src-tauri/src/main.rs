//! SerialBot desktop app entry point.

// Release builds on Windows must not open a console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    match serialbot_app::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            #[allow(clippy::print_stderr, reason = "last-resort startup error report")]
            {
                eprintln!("SerialBot failed to start: {e}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}
