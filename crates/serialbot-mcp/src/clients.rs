// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! Adding SerialBot to the configuration files of MCP clients (Claude
//! Desktop, Cursor, VS Code), shared by the app and the CLI.
//!
//! Only the `serialbot` entry is written: every other key, server and
//! setting in the file is kept, in its order. A file that is not plain JSON
//! (VS Code allows comments) is left alone and reported, so the caller can
//! show the entry for the user to paste instead.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// The name the entry is written under.
pub const ENTRY_NAME: &str = "serialbot";

/// An MCP client whose configuration file SerialBot can write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    /// `claude_desktop_config.json`; starts servers as child processes, so
    /// it gets the stdio bridge.
    ClaudeDesktop,
    /// `~/.cursor/mcp.json`; reaches the HTTP server with the token.
    Cursor,
    /// VS Code's user `mcp.json`; reaches the HTTP server with the token.
    VsCode,
}

/// Where the user's configuration lives: the system's config directory
/// (`%APPDATA%`, `~/Library/Application Support`, `~/.config`) and the home
/// directory.
#[derive(Debug, Clone)]
pub struct Dirs {
    /// The per-user configuration directory.
    pub config: PathBuf,
    /// The home directory.
    pub home: PathBuf,
}

/// How a client reaches SerialBot's server.
#[derive(Debug, Clone)]
pub struct Server {
    /// The HTTP endpoint, such as `http://127.0.0.1:9123/mcp`.
    pub url: String,
    /// The bearer token.
    pub token: String,
    /// The program and arguments of the stdio bridge, if there is one.
    pub stdio: Option<(String, Vec<String>)>,
}

/// Why an entry could not be written.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// The client starts servers itself and there is no bridge to start.
    #[error("{0} needs a program to start, and this build has none")]
    NoStdio(&'static str),
    /// The file exists but is not plain JSON (comments, trailing commas,
    /// a syntax error) or does not hold an object where one is expected.
    #[error("{path} is not plain JSON SerialBot can change ({reason}); add the entry by hand")]
    Unreadable {
        /// The file.
        path: String,
        /// What is wrong with it.
        reason: String,
    },
    /// The file does not exist and must not be created here (see
    /// [`install`]'s `create`).
    #[error("{0} does not exist yet; open the client once, or add the entry by hand")]
    Missing(String),
    /// Reading or writing failed.
    #[error("{path}: {reason}")]
    Io {
        /// The file.
        path: String,
        /// The system's message.
        reason: String,
    },
}

/// What [`install`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The configuration file.
    pub path: PathBuf,
    /// Where the previous content was saved, if the file existed and changed.
    pub backup: Option<PathBuf>,
    /// False when the file already held exactly this entry.
    pub changed: bool,
}

impl Client {
    /// Every client, in the order they are offered.
    pub const ALL: [Client; 3] = [Client::ClaudeDesktop, Client::Cursor, Client::VsCode];

    /// The id used on the command line and between the app's UI and backend.
    pub fn id(self) -> &'static str {
        match self {
            Client::ClaudeDesktop => "claude-desktop",
            Client::Cursor => "cursor",
            Client::VsCode => "vscode",
        }
    }

    /// The client's product name.
    pub fn name(self) -> &'static str {
        match self {
            Client::ClaudeDesktop => "Claude Desktop",
            Client::Cursor => "Cursor",
            Client::VsCode => "VS Code",
        }
    }

    /// The client for an [`id`](Self::id), ignoring case.
    pub fn parse(id: &str) -> Option<Client> {
        Client::ALL
            .into_iter()
            .find(|c| c.id().eq_ignore_ascii_case(id))
    }

    /// The configuration file the entry goes into.
    pub fn config_path(self, dirs: &Dirs) -> PathBuf {
        match self {
            Client::ClaudeDesktop => dirs
                .config
                .join("Claude")
                .join("claude_desktop_config.json"),
            Client::Cursor => dirs.home.join(".cursor").join("mcp.json"),
            Client::VsCode => dirs.config.join("Code").join("User").join("mcp.json"),
        }
    }

    /// The key that holds the servers.
    fn servers_key(self) -> &'static str {
        match self {
            Client::VsCode => "servers",
            Client::ClaudeDesktop | Client::Cursor => "mcpServers",
        }
    }

    /// The entry for `server` in this client's format.
    pub fn entry(self, server: &Server) -> Result<Value, Error> {
        let auth = format!("Bearer {}", server.token);
        Ok(match self {
            Client::ClaudeDesktop => {
                let (command, args) = server.stdio.as_ref().ok_or(Error::NoStdio(self.name()))?;
                json!({ "command": command, "args": args })
            }
            Client::Cursor => json!({ "url": server.url, "headers": { "Authorization": auth } }),
            Client::VsCode => json!({
                "type": "http",
                "url": server.url,
                "headers": { "Authorization": auth },
            }),
        })
    }

    /// The whole file a new configuration would hold: just the entry.
    pub fn snippet(self, server: &Server) -> Result<String, Error> {
        merge(None, self, &self.entry(server)?, Path::new(""))
    }
}

/// `existing` (the file's text, or `None` when there is none) with the
/// `serialbot` entry set to `entry`; other content is kept in order. Empty
/// or blank text counts as no file. `path` is only used in errors.
pub fn merge(
    existing: Option<&str>,
    client: Client,
    entry: &Value,
    path: &Path,
) -> Result<String, Error> {
    let unreadable = |reason: String| Error::Unreadable {
        path: path.display().to_string(),
        reason,
    };
    let mut root = match existing.map(str::trim).filter(|t| !t.is_empty()) {
        Some(text) => serde_json::from_str::<Value>(text).map_err(|e| unreadable(e.to_string()))?,
        None => Value::Object(Map::new()),
    };
    let Value::Object(top) = &mut root else {
        return Err(unreadable("the file is not a JSON object".to_owned()));
    };
    let servers = top
        .entry(client.servers_key())
        .or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(servers) = servers else {
        return Err(unreadable(format!(
            "\"{}\" is not an object",
            client.servers_key()
        )));
    };
    servers.insert(ENTRY_NAME.to_owned(), entry.clone());
    let mut text = serde_json::to_string_pretty(&root).map_err(|e| unreadable(e.to_string()))?;
    text.push('\n');
    Ok(text)
}

/// The backup [`install`] keeps of a changed file: `<file>.serialbot-backup`.
pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".serialbot-backup");
    path.with_file_name(name)
}

/// Writes the `serialbot` entry into `client`'s configuration file.
///
/// The previous content is saved next to it (see [`backup_path`]) before
/// the file is changed, and the file is rewritten in place rather than
/// replaced, so a packaged (MSIX) app changes the real file. When the file
/// does not exist it is created along with its folders only if `create` is
/// true: a packaged app should pass false for files under the config
/// directory, where Windows would keep a new file private to the app.
/// Writing the same entry again changes nothing.
pub fn install(
    client: Client,
    server: &Server,
    dirs: &Dirs,
    create: bool,
) -> Result<Installed, Error> {
    let path = client.config_path(dirs);
    let io = |e: std::io::Error| Error::Io {
        path: path.display().to_string(),
        reason: e.to_string(),
    };
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(io(e)),
    };
    if existing.is_none() && !create {
        return Err(Error::Missing(path.display().to_string()));
    }
    let text = merge(existing.as_deref(), client, &client.entry(server)?, &path)?;
    if existing.as_deref() == Some(text.as_str()) {
        return Ok(Installed {
            path,
            backup: None,
            changed: false,
        });
    }
    let backup = if let Some(old) = &existing {
        let backup = backup_path(&path);
        std::fs::write(&backup, old).map_err(io)?;
        Some(backup)
    } else {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        None
    };
    std::fs::write(&path, &text).map_err(io)?;
    Ok(Installed {
        path,
        backup,
        changed: true,
    })
}

/// The `create` argument of [`install`]: always for an unpackaged app; for
/// a packaged (MSIX) one only outside the config directory, since Windows
/// keeps files a packaged app creates there private to it, where the client
/// would never see them.
pub fn may_create(client: Client, dirs: &Dirs, packaged: bool) -> bool {
    !packaged || !client.config_path(dirs).starts_with(&dirs.config)
}

#[cfg(test)]
mod tests;
