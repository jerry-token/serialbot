// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { useEffect, useState } from "react";
import type { ClientConfig, McpClientId, McpSettings, McpStatus } from "../api";
import { claudeCommand, claudeDesktopConfig } from "../api";
import type { Strings } from "../i18n";

interface Props {
  t: Strings;
  status: McpStatus;
  onApply: (settings: McpSettings) => void;
  onRegenerate: () => void;
  /** Copies text to the clipboard; rejects if not allowed. */
  copy: (text: string) => Promise<void>;
  /** The settings to copy into a client's file. */
  onClientConfig: (client: McpClientId) => Promise<ClientConfig>;
}

const CLIENTS: { id: McpClientId; name: string }[] = [
  { id: "claude-desktop", name: "Claude Desktop" },
  { id: "cursor", name: "Cursor" },
  { id: "vscode", name: "VS Code" },
  { id: "codex", name: "Codex" },
  { id: "gemini", name: "Gemini CLI" },
  { id: "opencode", name: "opencode" },
];

export function McpPanel({ t, status, onApply, onRegenerate, copy, onClientConfig }: Props) {
  const [client, setClient] = useState<McpClientId | null>(null);
  const [config, setConfig] = useState<ClientConfig | null>(null);
  const [port, setPort] = useState(String(status.settings.port));
  const [token, setToken] = useState(status.settings.token);
  const [note, setNote] = useState<string | null>(null);

  // Show what the backend saved (for example after a new token).
  useEffect(() => {
    setPort(String(status.settings.port));
    setToken(status.settings.token);
  }, [status.settings.port, status.settings.token]);

  const save = () => {
    const n = Number(port);
    if (!Number.isInteger(n) || n < 1 || n > 65535) {
      setNote(t.invalidMcpPort);
      return;
    }
    setNote(null);
    onApply({ ...status.settings, port: n, token });
  };

  const command = status.url ? claudeCommand(status.url, status.settings.token) : null;
  const lanCommand = status.lanUrl ? claudeCommand(status.lanUrl, status.settings.token) : null;
  const lan = status.settings.listen === "lan";
  const desktop =
    status.url && status.stdioCommand ? claudeDesktopConfig(status.stdioCommand) : null;
  const copyNote = (text: string) =>
    copy(text).then(
      () => setNote(t.copied),
      (e: unknown) => setNote(String(e)),
    );

  return (
    <section className="card mcp-panel" aria-label={t.mcpTitle}>
      <h2>{t.mcpTitle}</h2>
      <label>
        <input
          type="checkbox"
          checked={status.settings.enabled}
          onChange={(e) => onApply({ ...status.settings, enabled: e.target.checked })}
        />
        {t.mcpEnable}
      </label>
      <label>
        <input
          type="checkbox"
          checked={status.settings.confirm}
          onChange={(e) => onApply({ ...status.settings, confirm: e.target.checked })}
        />
        {t.mcpConfirm}
      </label>
      <div className="row">
        <label>
          {t.mcpListen}
          <select
            aria-label={t.mcpListen}
            value={status.settings.listen}
            onChange={(e) =>
              onApply({ ...status.settings, listen: e.target.value === "lan" ? "lan" : "local" })
            }
          >
            <option value="local">{t.mcpListenLocal}</option>
            <option value="lan">{t.mcpListenLan}</option>
          </select>
        </label>
      </div>
      {lan && <p className="warning">{t.mcpLanWarning}</p>}
      <div className="row">
        <label>
          {t.mcpPort}
          <input
            aria-label={t.mcpPort}
            className="mcp-port"
            inputMode="numeric"
            value={port}
            onChange={(e) => setPort(e.target.value)}
          />
        </label>
        <label className="grow">
          {t.mcpToken}
          <input
            aria-label={t.mcpToken}
            className="mcp-token"
            value={token}
            onChange={(e) => setToken(e.target.value)}
          />
        </label>
        <button type="button" onClick={onRegenerate}>
          {t.mcpRegenerate}
        </button>
        <button type="button" className="primary" onClick={save}>
          {t.mcpSave}
        </button>
      </div>
      <p className={status.running ? "ok" : "muted"}>
        {status.url ? t.mcpRunning(status.url) : t.mcpStopped}
      </p>
      {command && (
        <div className="row">
          <span className="muted">{t.mcpClaude}</span>
          <code className="grow">{command}</code>
          <button type="button" aria-label={t.copyCommand} onClick={() => copyNote(command)}>
            {t.copy}
          </button>
        </div>
      )}
      {lan && status.running && (
        <p className={status.lanUrl ? "ok" : "muted"}>
          {status.lanUrl ? t.mcpLanRunning(status.lanUrl) : t.mcpLanUnknown}
        </p>
      )}
      {lanCommand && (
        <div className="row">
          <span className="muted">{t.mcpClaudeLan}</span>
          <code className="grow">{lanCommand}</code>
          <button type="button" aria-label={t.copyLanCommand} onClick={() => copyNote(lanCommand)}>
            {t.copy}
          </button>
        </div>
      )}
      <div className="row mcp-clients">
        <span className="muted">{t.mcpConfigFor}</span>
        {CLIENTS.map((c) => (
          <button
            key={c.id}
            type="button"
            aria-pressed={client === c.id}
            onClick={() => {
              setNote(null);
              setClient(c.id);
              setConfig(null);
              onClientConfig(c.id).then(setConfig, (e: unknown) => setNote(String(e)));
            }}
          >
            {c.name}
          </button>
        ))}
      </div>
      {config?.error && <p className="error">{config.error}</p>}
      {config?.snippet && (
        <>
          {config.path && <p className="muted path">{t.mcpDefaultPath(config.path)}</p>}
          <p className="muted">{t.mcpConfigHint(config.client, config.section)}</p>
          <div className="row">
            <pre className="grow config">{config.snippet}</pre>
            <button
              type="button"
              aria-label={t.copyClientConfig(config.client)}
              onClick={() => copyNote(config.snippet ?? "")}
            >
              {t.copy}
            </button>
          </div>
        </>
      )}
      {desktop && (
        <div className="row">
          <span className="muted">{t.mcpDesktop}</span>
          <pre className="grow config">{desktop}</pre>
          <button type="button" aria-label={t.copyDesktop} onClick={() => copyNote(desktop)}>
            {t.copy}
          </button>
        </div>
      )}
      {status.error && <p className="error">{status.error}</p>}
      {note && <p className="note">{note}</p>}
    </section>
  );
}
