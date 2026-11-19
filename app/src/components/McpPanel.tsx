import { useEffect, useState } from "react";
import type { McpSettings, McpStatus } from "../api";
import { claudeCommand, claudeDesktopConfig } from "../api";
import type { Strings } from "../i18n";

interface Props {
  t: Strings;
  status: McpStatus;
  onApply: (settings: McpSettings) => void;
  onRegenerate: () => void;
  /** Copies text to the clipboard; rejects if not allowed. */
  copy: (text: string) => Promise<void>;
}

export function McpPanel({ t, status, onApply, onRegenerate, copy }: Props) {
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
