// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
use std::path::Path;

use super::*;

fn server() -> Server {
    Server {
        url: "http://127.0.0.1:9123/mcp".to_owned(),
        token: "tok".to_owned(),
        stdio: Some(("/opt/serialbot".to_owned(), vec!["mcp".to_owned()])),
    }
}

#[test]
fn ids_names_and_paths() {
    for c in Client::ALL {
        assert_eq!(Client::parse(c.id()), Some(c));
    }
    assert_eq!(Client::parse("VSCode"), Some(Client::VsCode));
    assert_eq!(Client::parse("zed"), None);
    let dirs = Dirs {
        config: PathBuf::from("/c"),
        home: PathBuf::from("/h"),
    };
    assert_eq!(
        Client::ClaudeDesktop.config_path(&dirs),
        Path::new("/c/Claude/claude_desktop_config.json")
    );
    assert_eq!(
        Client::Cursor.config_path(&dirs),
        Path::new("/h/.cursor/mcp.json")
    );
    assert_eq!(
        Client::VsCode.config_path(&dirs),
        Path::new("/c/Code/User/mcp.json")
    );
    assert_eq!(Client::VsCode.name(), "VS Code");
}

#[test]
fn each_client_gets_its_own_format() {
    let s = server();
    assert_eq!(
        Client::ClaudeDesktop.entry(&s).unwrap(),
        json!({ "command": "/opt/serialbot", "args": ["mcp"] })
    );
    assert_eq!(
        Client::Cursor.entry(&s).unwrap(),
        json!({ "url": s.url, "headers": { "Authorization": "Bearer tok" } })
    );
    let vscode = Client::VsCode.snippet(&s).unwrap();
    let value: Value = serde_json::from_str(&vscode).unwrap();
    assert_eq!(value["servers"]["serialbot"]["type"], "http");
    assert_eq!(
        value["servers"]["serialbot"]["headers"]["Authorization"],
        "Bearer tok"
    );
    let no_bridge = Server { stdio: None, ..s };
    assert_eq!(
        Client::ClaudeDesktop.entry(&no_bridge),
        Err(Error::NoStdio("Claude Desktop"))
    );
}

#[test]
fn codex_gemini_and_opencode_paths_and_entries() {
    let dirs = Dirs {
        config: PathBuf::from("/c"),
        home: PathBuf::from("/h"),
    };
    assert_eq!(
        Client::Codex.config_path(&dirs),
        Path::new("/h/.codex/config.toml")
    );
    assert_eq!(
        Client::Gemini.config_path(&dirs),
        Path::new("/h/.gemini/settings.json")
    );
    assert_eq!(
        Client::OpenCode.config_path(&dirs),
        Path::new("/h/.config/opencode/opencode.json")
    );
    let s = server();
    let gemini: Value = serde_json::from_str(&Client::Gemini.snippet(&s).unwrap()).unwrap();
    assert_eq!(
        gemini,
        json!({ "mcpServers": { "serialbot": {
            "httpUrl": s.url, "headers": { "Authorization": "Bearer tok" }
        } } })
    );
    let opencode: Value = serde_json::from_str(&Client::OpenCode.snippet(&s).unwrap()).unwrap();
    assert_eq!(
        opencode["mcp"]["servers"]["serialbot"],
        json!({
            "type": "remote", "url": s.url, "oauth": false,
            "headers": { "Authorization": "Bearer tok" }
        })
    );
    assert_eq!(
        Client::Codex.snippet(&s).unwrap(),
        "[mcp_servers.serialbot]\nurl = \"http://127.0.0.1:9123/mcp\"\nhttp_headers = { Authorization = \"Bearer tok\" }\n"
    );
}

#[test]
fn json_snippets_parse_and_hold_only_serialbot() {
    let s = server();
    for c in Client::ALL {
        if c == Client::Codex {
            continue;
        }
        let text = c.snippet(&s).unwrap();
        assert!(text.ends_with("}\n"), "{c:?}");
        let mut value: Value = serde_json::from_str(&text).unwrap();
        for key in c.servers_path() {
            let obj = value.as_object().unwrap();
            assert_eq!(obj.len(), 1, "{c:?}");
            value = obj[*key].clone();
        }
        assert_eq!(value.as_object().unwrap().len(), 1);
        assert_eq!(value[ENTRY_NAME], c.entry(&s).unwrap(), "{c:?}");
    }
    let no_bridge = Server { stdio: None, ..s };
    assert!(Client::ClaudeDesktop.snippet(&no_bridge).is_err());
    assert!(Client::Cursor.snippet(&no_bridge).is_ok());
    assert_eq!(Client::OpenCode.section(), "mcp.servers");
    assert_eq!(Client::Codex.section(), "[mcp_servers]");
}
