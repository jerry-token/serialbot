// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
use super::*;

/// A fresh folder standing in for the config and home directories.
struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("serialbot-clients-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }

    fn dirs(&self) -> Dirs {
        Dirs {
            config: self.0.join("config"),
            home: self.0.join("home"),
        }
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

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
fn merging_keeps_everything_else_in_order() {
    let existing = r#"{
  "theme": "dark",
  "mcpServers": {
    "zeta": { "command": "z" },
    "serialbot": { "command": "old" },
    "alpha": { "command": "a" }
  },
  "after": 1
}"#;
    let entry = json!({ "command": "new", "args": ["mcp"] });
    let text = merge(
        Some(existing),
        Client::ClaudeDesktop,
        &entry,
        Path::new("f"),
    )
    .unwrap();
    let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(keys(&value), ["theme", "mcpServers", "after"]);
    assert_eq!(keys(&value["mcpServers"]), ["zeta", "serialbot", "alpha"]);
    assert_eq!(value["mcpServers"]["serialbot"], entry);
    assert!(text.ends_with("}\n"));
    // No file, or a blank one, starts a new object.
    for empty in [None, Some(""), Some("  \n")] {
        let text = merge(empty, Client::Cursor, &entry, Path::new("f")).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(keys(&value), ["mcpServers"]);
    }
}

#[test]
fn files_that_are_not_plain_json_objects_are_refused() {
    let entry = json!({});
    for bad in [
        "// comment\n{}",
        "{\"servers\": {},}",
        "[1, 2]",
        "{\"servers\": []}",
    ] {
        let err = merge(Some(bad), Client::VsCode, &entry, Path::new("/x/mcp.json")).unwrap_err();
        assert!(
            matches!(&err, Error::Unreadable { path, .. } if path == "/x/mcp.json"),
            "{bad}: {err:?}"
        );
        assert!(err.to_string().contains("by hand"));
    }
}

#[test]
fn install_creates_backs_up_and_is_idempotent() {
    let temp = Temp::new("install");
    let dirs = temp.dirs();
    let s = server();

    // A new file, with its folders.
    let first = install(Client::Cursor, &s, &dirs, true).unwrap();
    assert_eq!(first.path, dirs.home.join(".cursor/mcp.json"));
    assert_eq!((first.changed, first.backup.as_ref()), (true, None));
    let written = std::fs::read_to_string(&first.path).unwrap();
    assert!(written.contains("Bearer tok"));

    // The same entry again: nothing changes, no backup.
    let again = install(Client::Cursor, &s, &dirs, true).unwrap();
    assert_eq!((again.changed, again.backup), (false, None));

    // A new token: the old file is kept as the backup.
    let other = Server {
        token: "new".to_owned(),
        ..s.clone()
    };
    let changed = install(Client::Cursor, &other, &dirs, false).unwrap();
    let backup = changed.backup.unwrap();
    assert_eq!(backup, dirs.home.join(".cursor/mcp.json.serialbot-backup"));
    assert_eq!(std::fs::read_to_string(backup).unwrap(), written);
    assert!(
        std::fs::read_to_string(&changed.path)
            .unwrap()
            .contains("Bearer new")
    );
}

#[test]
fn install_refuses_to_create_when_asked_and_leaves_bad_files_alone() {
    let temp = Temp::new("refuse");
    let dirs = temp.dirs();
    let s = server();
    let err = install(Client::VsCode, &s, &dirs, false).unwrap_err();
    assert!(matches!(err, Error::Missing(_)));
    assert!(!Client::VsCode.config_path(&dirs).exists());

    let path = Client::VsCode.config_path(&dirs);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{ // mine\n}").unwrap();
    assert!(matches!(
        install(Client::VsCode, &s, &dirs, true),
        Err(Error::Unreadable { .. })
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ // mine\n}");
    assert!(!backup_path(&path).exists());

    // A folder where the file should be cannot be read.
    let claude = Client::ClaudeDesktop.config_path(&dirs);
    std::fs::create_dir_all(&claude).unwrap();
    assert!(matches!(
        install(Client::ClaudeDesktop, &s, &dirs, true),
        Err(Error::Io { .. })
    ));
}

#[test]
fn packaged_apps_create_files_only_outside_the_config_directory() {
    let dirs = Dirs {
        config: PathBuf::from("/c"),
        home: PathBuf::from("/h"),
    };
    assert!(may_create(Client::VsCode, &dirs, false));
    assert!(!may_create(Client::VsCode, &dirs, true));
    assert!(!may_create(Client::ClaudeDesktop, &dirs, true));
    assert!(may_create(Client::Cursor, &dirs, true));
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
    // None of them is under the config directory: a packaged app may create them.
    for c in [Client::Codex, Client::Gemini, Client::OpenCode] {
        assert!(may_create(c, &dirs, true), "{c:?}");
    }
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
fn opencode_keeps_the_layout_its_file_already_uses() {
    let entry = json!({ "type": "remote" });
    let v1 = r#"{ "mcp": { "other": { "type": "local", "command": ["x"] } } }"#;
    let text = merge(Some(v1), Client::OpenCode, &entry, Path::new("f")).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["mcp"]["serialbot"], entry);
    assert!(value["mcp"].get("servers").is_none());

    let v2 = r#"{ "mcp": { "servers": { "other": { "type": "local" } } }, "theme": "x" }"#;
    let text = merge(Some(v2), Client::OpenCode, &entry, Path::new("f")).unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["mcp"]["servers"]["serialbot"], entry);
    assert_eq!(value["mcp"]["servers"]["other"]["type"], "local");
    assert_eq!(value["theme"], "x");

    // An "mcp" without servers in either layout: the current one.
    let empty = r#"{ "mcp": {} }"#;
    let value: Value = serde_json::from_str(
        &merge(Some(empty), Client::OpenCode, &entry, Path::new("f")).unwrap(),
    )
    .unwrap();
    assert_eq!(value["mcp"]["servers"]["serialbot"], entry);
}

#[test]
fn codex_toml_keeps_comments_and_other_servers() {
    let existing = "# my settings\nmodel = \"o4\"\n\n[mcp_servers.other]\ncommand = \"x\" # keep me\n\n[profiles.fast]\nmodel = \"mini\"\n";
    let entry = Client::Codex.entry(&server()).unwrap();
    let text = merge(Some(existing), Client::Codex, &entry, Path::new("f")).unwrap();
    assert!(
        text.starts_with("# my settings\nmodel = \"o4\"\n"),
        "{text}"
    );
    assert!(text.contains("command = \"x\" # keep me"), "{text}");
    assert!(text.contains("[profiles.fast]"), "{text}");
    let parsed: toml_edit::DocumentMut = text.parse().unwrap();
    assert_eq!(
        parsed["mcp_servers"]["serialbot"]["url"].as_str(),
        Some("http://127.0.0.1:9123/mcp")
    );
    assert_eq!(
        parsed["mcp_servers"]["serialbot"]["http_headers"]["Authorization"].as_str(),
        Some("Bearer tok")
    );
    // Writing it again replaces the entry rather than adding another.
    let again = merge(Some(&text), Client::Codex, &entry, Path::new("f")).unwrap();
    assert_eq!(again, text);

    for bad in ["model = ", "mcp_servers = 3\n"] {
        let err = merge(
            Some(bad),
            Client::Codex,
            &entry,
            Path::new("/x/config.toml"),
        )
        .unwrap_err();
        assert!(matches!(err, Error::Unreadable { .. }), "{bad}: {err:?}");
    }
}

#[test]
fn install_writes_codex_toml() {
    let temp = Temp::new("codex");
    let dirs = temp.dirs();
    let done = install(Client::Codex, &server(), &dirs, true).unwrap();
    assert_eq!(done.path, dirs.home.join(".codex/config.toml"));
    assert!(
        std::fs::read_to_string(&done.path)
            .unwrap()
            .contains("[mcp_servers.serialbot]")
    );
    assert!(
        !install(Client::Codex, &server(), &dirs, true)
            .unwrap()
            .changed
    );
}
