// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! The CLI's own settings (`cli.json` in the app's config directory, next
//! to the app's `mcp.json`, never shared with it) and the app's MCP
//! settings, read only to reach a running app.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The app identifier: the config directory is named after it.
pub const IDENTIFIER: &str = "cc.aiany.serialbot";

/// The directory both files live in, or `None` without a config directory.
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join(IDENTIFIER))
}

/// What `cli.json` holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliConfig {
    /// The MCP token `mcp serve` uses and `mcp stdio` sends; created the
    /// first time `mcp serve` runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_token: Option<String>,
    /// The port `mcp serve` last asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_port: Option<u16>,
}

impl CliConfig {
    /// `cli.json` in `dir`.
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("cli.json")
    }

    /// Reads `cli.json`; a missing file is the default, an unreadable or
    /// invalid one is an error naming the file.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = Self::path(dir);
        match std::fs::read(&path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Writes `cli.json`, creating the directory.
    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let path = Self::path(dir);
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(&path, text + "\n"))
            .map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// The running app's MCP server, from its `mcp.json`: port and token, or
/// `None` when the file is missing, invalid or the server is off.
pub fn app_mcp(dir: &Path) -> Option<(u16, String)> {
    let bytes = std::fs::read(dir.join("mcp.json")).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    if value["enabled"].as_bool() != Some(true) {
        return None;
    }
    let port = u16::try_from(value["port"].as_u64()?).ok()?;
    let token = value["token"].as_str()?.to_owned();
    Some((port, token))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "serialbot-cli-config-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn saves_and_loads() {
        let d = dir("roundtrip");
        assert_eq!(
            CliConfig::load(&d),
            Ok(CliConfig::default()),
            "missing is the default"
        );
        let config = CliConfig {
            mcp_token: Some("t".repeat(20)),
            mcp_port: Some(9700),
        };
        config.save(&d).unwrap();
        assert_eq!(CliConfig::load(&d), Ok(config));
        std::fs::write(CliConfig::path(&d), "{oops").unwrap();
        assert!(CliConfig::load(&d).unwrap_err().contains("cli.json"));
        std::fs::remove_dir_all(&d).unwrap();
        std::fs::write(&d, "a file, not a directory").unwrap();
        assert!(CliConfig::default().save(&d).is_err());
        std::fs::remove_file(&d).unwrap();
        assert!(config_dir().is_none_or(|p| p.ends_with(IDENTIFIER)));
    }

    #[test]
    fn reads_the_apps_server_only_when_on() {
        let d = dir("app");
        assert_eq!(app_mcp(&d), None);
        std::fs::create_dir_all(&d).unwrap();
        let file = d.join("mcp.json");
        std::fs::write(&file, r#"{"enabled": true, "port": 9633, "token": "abc"}"#).unwrap();
        assert_eq!(app_mcp(&d), Some((9633, "abc".into())));
        std::fs::write(&file, r#"{"enabled": false, "port": 9633, "token": "abc"}"#).unwrap();
        assert_eq!(app_mcp(&d), None);
        std::fs::write(&file, r#"{"enabled": true, "port": 99999, "token": "abc"}"#).unwrap();
        assert_eq!(app_mcp(&d), None);
        std::fs::write(&file, "nope").unwrap();
        assert_eq!(app_mcp(&d), None);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
