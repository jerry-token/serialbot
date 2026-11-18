//! Persistent MCP server settings (`mcp.json` in the app config directory).

use std::path::Path;

use serde::{Deserialize, Serialize};
use serialbot_mcp::{DEFAULT_PORT, McpOptions, generate_token};

/// What the user chose for the embedded MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSettings {
    /// Whether the server runs. Off until the user turns it on.
    pub enabled: bool,
    /// Preferred port; if it is taken the next free one of the following
    /// nine is used.
    pub port: u16,
    /// Bearer token clients must send.
    pub token: String,
}

impl McpSettings {
    /// Disabled, default port, a fresh random token.
    pub fn new_default() -> Result<Self, String> {
        Ok(Self {
            enabled: false,
            port: DEFAULT_PORT,
            token: generate_token().map_err(|e| format!("cannot generate a token: {e}"))?,
        })
    }

    /// Checks the values the server will be started with.
    pub fn validate(&self) -> Result<(), String> {
        if self.port == 0 {
            return Err("the port must be between 1 and 65535".into());
        }
        McpOptions::new(self.token.clone())
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// Loads settings from `path`.
///
/// A missing, unreadable or invalid file yields fresh defaults, which are
/// saved so the token stays the same next time. The returned message, if
/// any, explains why the file was not used.
pub fn load(path: &Path) -> Result<(McpSettings, Option<String>), String> {
    let problem = match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<McpSettings>(&bytes) {
            Ok(settings) => match settings.validate() {
                Ok(()) => return Ok((settings, None)),
                Err(e) => Some(format!("ignored invalid MCP settings: {e}")),
            },
            Err(e) => Some(format!("ignored unreadable MCP settings: {e}")),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => Some(format!("cannot read MCP settings: {e}")),
    };
    let settings = McpSettings::new_default()?;
    save(path, &settings)?;
    Ok((settings, problem))
}

/// Writes settings to `path`, creating its directory. On Unix the file is
/// readable by its owner only, because it holds the token.
pub fn save(path: &Path, settings: &McpSettings) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let json = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    let failed = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
    std::fs::write(path, json).map_err(failed)?;
    // The file holds the token: only its owner may read it.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(failed)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A fresh directory under the system temp dir, removed on drop.
    pub(crate) struct TempDir(pub(crate) PathBuf);

    impl TempDir {
        /// `name` must be unique among the tests; the process id keeps
        /// parallel test runs apart. Only characters valid on every OS.
        pub(crate) fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("serialbot-test-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn defaults_are_off_with_a_valid_token() {
        let settings = McpSettings::new_default().unwrap();
        assert!(!settings.enabled);
        assert_eq!(settings.port, DEFAULT_PORT);
        settings.validate().unwrap();
        assert_ne!(settings.token, McpSettings::new_default().unwrap().token);
    }

    #[test]
    fn validation() {
        let good = McpSettings::new_default().unwrap();
        let zero = McpSettings {
            port: 0,
            ..good.clone()
        };
        assert!(zero.validate().unwrap_err().contains("port"));
        let short = McpSettings {
            token: "short".into(),
            ..good
        };
        assert!(short.validate().unwrap_err().contains("at least"));
    }

    #[test]
    fn missing_file_creates_defaults_that_persist() {
        let dir = TempDir::new("missing");
        let path = dir.0.join("nested/mcp.json");
        let (first, problem) = load(&path).unwrap();
        assert_eq!(problem, None);
        let (second, _) = load(&path).unwrap();
        assert_eq!(first, second, "the token is stable across restarts");
    }

    #[test]
    fn saved_settings_round_trip() {
        let dir = TempDir::new("roundtrip");
        let path = dir.0.join("mcp.json");
        let settings = McpSettings {
            enabled: true,
            port: 9700,
            token: "my-own-token-123456".into(),
        };
        save(&path, &settings).unwrap();
        assert_eq!(load(&path).unwrap(), (settings, None));
        let json = std::fs::read_to_string(&path).unwrap();
        assert!(json.contains("\"enabled\": true"), "{json}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "only the owner can read the token");
        }
    }

    #[test]
    fn bad_files_are_replaced_with_defaults() {
        let dir = TempDir::new("bad");
        let path = dir.0.join("mcp.json");
        std::fs::write(&path, b"{not json").unwrap();
        let (settings, problem) = load(&path).unwrap();
        assert!(problem.unwrap().contains("unreadable"));
        assert!(!settings.enabled);

        std::fs::write(&path, br#"{"enabled":true,"port":9633,"token":"x"}"#).unwrap();
        let (_, problem) = load(&path).unwrap();
        assert!(problem.unwrap().contains("invalid"));

        // A directory where the file should be cannot be read or replaced.
        let blocked = dir.0.join("blocked");
        std::fs::create_dir_all(&blocked).unwrap();
        assert!(load(&blocked).is_err());
    }
}
