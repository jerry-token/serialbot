// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! SerialBot desktop app entry point.

// Release builds on Windows must not open a console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    // `serialbot mcp`: stdio bridge for MCP clients, no window.
    if std::env::args().nth(1).as_deref() == Some("mcp") {
        return match serialbot_app::cli::mcp_bridge() {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                #[allow(clippy::print_stderr, reason = "MCP clients show the server's stderr")]
                {
                    eprintln!("serialbot mcp: {e}");
                }
                std::process::ExitCode::FAILURE
            }
        };
    }
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
