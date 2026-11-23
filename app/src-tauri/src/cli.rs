// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! `serialbot mcp`: the stdio MCP bridge for clients that start their MCP
//! servers as child processes (Claude Desktop). It runs no window: it reads
//! the running app's MCP settings and forwards to its HTTP server.

use std::path::{Path, PathBuf};

use crate::mcp::PORT_ATTEMPTS;
use crate::settings::McpSettings;

/// The app identifier from `tauri.conf.json`; the settings live in the
/// config directory named after it.
pub const IDENTIFIER: &str = "cc.aiany.serialbot";

/// Where the app keeps `mcp.json` (the same place Tauri's
/// `app_config_dir` resolves to), or `None` if the system has no config
/// directory.
pub fn settings_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(IDENTIFIER).join("mcp.json"))
}

/// Reads the settings the bridge needs, without creating or changing the
/// file. Fails with a message for the user when the app has never run, the
/// file is unreadable, or the MCP server is turned off.
pub fn read_settings(path: &Path) -> Result<McpSettings, String> {
    let bytes = std::fs::read(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "SerialBot has not been started yet: open it once and turn the MCP server on in its AI page".to_owned()
        } else {
            format!("cannot read {}: {e}", path.display())
        }
    })?;
    let settings: McpSettings = serde_json::from_slice(&bytes)
        .map_err(|e| format!("cannot use {}: {e}", path.display()))?;
    settings.validate()?;
    if !settings.enabled {
        return Err("SerialBot's MCP server is off: turn it on in the app's AI page".to_owned());
    }
    Ok(settings)
}

/// URLs the server may listen on: the preferred port and the fallbacks.
pub fn bridge_urls(settings: &McpSettings) -> Vec<String> {
    (0..PORT_ATTEMPTS)
        .filter_map(|i| settings.port.checked_add(i))
        .map(|port| format!("http://127.0.0.1:{port}{}", serialbot_mcp::MCP_PATH))
        .collect()
}

/// The path a stdio MCP client should start, with the argument `mcp`.
/// Inside an AppImage the running file is a temporary mount, so the
/// AppImage itself is given.
pub fn launcher() -> Option<String> {
    std::env::var("APPIMAGE")
        .ok()
        .filter(|p| !p.is_empty())
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .map(|p| p.display().to_string())
        })
}

/// Runs the bridge on stdin/stdout until the client closes it.
pub fn mcp_bridge() -> Result<(), String> {
    let path = settings_path().ok_or("this system has no configuration directory")?;
    let settings = read_settings(&path)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start: {e}"))?;
    runtime
        .block_on(serialbot_mcp::bridge::run_stdio(
            &bridge_urls(&settings),
            &settings.token,
        ))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::tests::TempDir;

    #[test]
    fn identifier_matches_the_app_config() {
        let conf = include_str!("../tauri.conf.json");
        assert!(conf.contains(&format!("\"identifier\": \"{IDENTIFIER}\"")));
        let path = settings_path().unwrap();
        assert!(path.ends_with(Path::new(IDENTIFIER).join("mcp.json")));
    }

    #[test]
    fn settings_are_read_without_being_created() {
        let dir = TempDir::new("cli-settings");
        let path = dir.0.join("mcp.json");
        let missing = read_settings(&path).unwrap_err();
        assert!(missing.contains("not been started"), "{missing}");
        assert!(!path.exists(), "the bridge never writes the file");

        std::fs::write(&path, b"{not json").unwrap();
        assert!(read_settings(&path).unwrap_err().contains("cannot use"));

        let mut settings = McpSettings::new_default().unwrap();
        crate::settings::save(&path, &settings).unwrap();
        assert!(read_settings(&path).unwrap_err().contains("is off"));
        settings.enabled = true;
        crate::settings::save(&path, &settings).unwrap();
        assert_eq!(read_settings(&path).unwrap(), settings);

        settings.port = 0;
        std::fs::write(&path, serde_json::to_vec(&settings).unwrap()).unwrap();
        assert!(read_settings(&path).is_err(), "invalid values are refused");
        assert!(read_settings(&dir.0).unwrap_err().contains("cannot read"));
    }

    #[test]
    fn urls_cover_the_fallback_ports() {
        let mut settings = McpSettings::new_default().unwrap();
        settings.port = 7310;
        let urls = bridge_urls(&settings);
        assert_eq!(urls.len(), usize::from(PORT_ATTEMPTS));
        assert_eq!(urls[0], "http://127.0.0.1:7310/mcp");
        assert_eq!(urls[9], "http://127.0.0.1:7319/mcp");
        settings.port = u16::MAX - 1;
        assert_eq!(bridge_urls(&settings).len(), 2, "no wrap-around");
    }

    #[test]
    fn the_launcher_is_this_executable() {
        assert!(launcher().is_some());
    }
}
