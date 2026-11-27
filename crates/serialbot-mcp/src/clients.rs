// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
//! The settings that connect MCP clients (Claude Desktop, Cursor, VS Code,
//! Codex, Gemini CLI, opencode) to SerialBot, shared by the app and the CLI.
//!
//! SerialBot only shows them, with the file each client reads by default:
//! the user copies the `serialbot` entry into the file their client really
//! uses. Nothing here writes a file: paths differ between installs
//! (Insiders builds, portable installs, project files, `CODEX_HOME`,
//! `XDG_CONFIG_HOME`), and another program's settings are the user's to
//! change.

use std::path::PathBuf;

use serde_json::{Map, Value, json};

/// The name the entry goes under.
pub const ENTRY_NAME: &str = "serialbot";

/// An MCP client SerialBot can show settings for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Client {
    /// `claude_desktop_config.json`; starts servers as child processes, so
    /// it gets the stdio bridge.
    ClaudeDesktop,
    /// `~/.cursor/mcp.json`; reaches the HTTP server with the token.
    Cursor,
    /// VS Code's user `mcp.json`; reaches the HTTP server with the token.
    VsCode,
    /// Codex (CLI and IDE extension): `~/.codex/config.toml`, HTTP.
    Codex,
    /// Gemini CLI: `~/.gemini/settings.json`, HTTP.
    Gemini,
    /// opencode: `~/.config/opencode/opencode.json`, HTTP, under
    /// `mcp.servers` (opencode's V2 config; older versions list servers
    /// directly under `mcp`).
    OpenCode,
}

/// The user's directories the default paths start from: the system's
/// config directory (`%APPDATA%`, `~/Library/Application Support`,
/// `~/.config`) and the home directory.
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

/// Why no settings can be shown.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// The client starts servers itself and there is no bridge to start.
    #[error("{0} needs a program to start, and this build has none")]
    NoStdio(&'static str),
}

impl Client {
    /// Every client, in the order they are offered.
    pub const ALL: [Client; 6] = [
        Client::ClaudeDesktop,
        Client::Cursor,
        Client::VsCode,
        Client::Codex,
        Client::Gemini,
        Client::OpenCode,
    ];

    /// The id used on the command line and between the app's UI and backend.
    pub fn id(self) -> &'static str {
        match self {
            Client::ClaudeDesktop => "claude-desktop",
            Client::Cursor => "cursor",
            Client::VsCode => "vscode",
            Client::Codex => "codex",
            Client::Gemini => "gemini",
            Client::OpenCode => "opencode",
        }
    }

    /// The client's product name.
    pub fn name(self) -> &'static str {
        match self {
            Client::ClaudeDesktop => "Claude Desktop",
            Client::Cursor => "Cursor",
            Client::VsCode => "VS Code",
            Client::Codex => "Codex",
            Client::Gemini => "Gemini CLI",
            Client::OpenCode => "opencode",
        }
    }

    /// The client for an [`id`](Self::id), ignoring case.
    pub fn parse(id: &str) -> Option<Client> {
        Client::ALL
            .into_iter()
            .find(|c| c.id().eq_ignore_ascii_case(id))
    }

    /// The file the client reads by default on this system (it may use
    /// another: see the module docs).
    pub fn config_path(self, dirs: &Dirs) -> PathBuf {
        match self {
            Client::ClaudeDesktop => dirs
                .config
                .join("Claude")
                .join("claude_desktop_config.json"),
            Client::Cursor => dirs.home.join(".cursor").join("mcp.json"),
            Client::VsCode => dirs.config.join("Code").join("User").join("mcp.json"),
            Client::Codex => dirs.home.join(".codex").join("config.toml"),
            Client::Gemini => dirs.home.join(".gemini").join("settings.json"),
            Client::OpenCode => dirs
                .home
                .join(".config")
                .join("opencode")
                .join("opencode.json"),
        }
    }

    /// Where the servers sit in the client's file, as the user would look
    /// for it: `mcpServers`, `servers`, `mcp.servers` or Codex's
    /// `mcp_servers` table. A file that already has it gets only the
    /// `serialbot` entry added inside.
    pub fn section(self) -> &'static str {
        match self {
            Client::Codex => "[mcp_servers]",
            Client::VsCode => "servers",
            Client::OpenCode => "mcp.servers",
            Client::ClaudeDesktop | Client::Cursor | Client::Gemini => "mcpServers",
        }
    }

    /// The keys that hold the servers in the client's file.
    fn servers_path(self) -> &'static [&'static str] {
        match self {
            Client::VsCode => &["servers"],
            Client::OpenCode => &["mcp", "servers"],
            Client::ClaudeDesktop | Client::Cursor | Client::Gemini | Client::Codex => {
                &["mcpServers"]
            }
        }
    }

    /// The `serialbot` entry for `server` in this client's format; fails
    /// for Claude Desktop when `server` has no stdio bridge.
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
            Client::Codex => {
                json!({ "url": server.url, "http_headers": { "Authorization": auth } })
            }
            Client::Gemini => {
                json!({ "httpUrl": server.url, "headers": { "Authorization": auth } })
            }
            Client::OpenCode => json!({
                "type": "remote",
                "url": server.url,
                "oauth": false,
                "headers": { "Authorization": auth },
            }),
        })
    }

    /// What to put in the client's file: a whole file holding only the
    /// `serialbot` entry (pretty JSON, or TOML for Codex). For a file that
    /// already has servers, the user adds just the entry.
    pub fn snippet(self, server: &Server) -> Result<String, Error> {
        let entry = self.entry(server)?;
        if self == Client::Codex {
            return Ok(codex_toml(&entry));
        }
        let mut value = json!({ ENTRY_NAME: entry });
        for key in self.servers_path().iter().rev() {
            let mut outer = Map::new();
            outer.insert((*key).to_owned(), value);
            value = Value::Object(outer);
        }
        let mut text = serde_json::to_string_pretty(&value).unwrap_or_default();
        text.push('\n');
        Ok(text)
    }
}

/// Codex's `config.toml` holding just `[mcp_servers.serialbot]`, set from
/// `entry` (strings, and tables of strings as inline tables).
fn codex_toml(entry: &Value) -> String {
    use toml_edit::{DocumentMut, InlineTable, Item, Table, value};
    let mut table = Table::new();
    for (key, v) in entry.as_object().into_iter().flatten() {
        match v {
            Value::String(s) => {
                table.insert(key, value(s.as_str()));
            }
            Value::Object(map) => {
                let mut inline = InlineTable::new();
                for (k, v) in map {
                    inline.insert(k, v.as_str().unwrap_or_default().into());
                }
                table.insert(key, value(inline));
            }
            _ => {}
        }
    }
    let mut servers = Table::new();
    servers.set_implicit(true);
    servers.insert(ENTRY_NAME, Item::Table(table));
    let mut doc = DocumentMut::new();
    doc.insert("mcp_servers", Item::Table(servers));
    doc.to_string()
}

#[cfg(test)]
mod tests;
