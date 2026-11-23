// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { useCallback, useEffect, useState } from "react";
import { type Backend, errorMessage, type SessionInfo } from "./api";
import { browserStorage } from "./commands";
import { CommandPanel } from "./components/CommandPanel";
import { type Lang, strings } from "./i18n";
import { applyAppearance, loadPrefs, resolveLanguage } from "./prefs";
import { useCommandRunner } from "./useCommandRunner";
import { useSharedGroups } from "./useSharedGroups";

interface Props {
  backend: Backend;
  lang: Lang;
  storage?: Storage | null;
}

/**
 * The command list in its own window, beside the main one. It has its own
 * pick of the open session to send to, and shares the saved groups with
 * the main window through storage (each window follows the other's edits).
 * Text commands are sent as UTF-8.
 */
export function CommandWindow({ backend, lang, storage = browserStorage() }: Props) {
  const [prefs, setPrefs] = useState(() => loadPrefs(storage));
  const t = strings(resolveLanguage(prefs.language, lang));
  const [groups, change] = useSharedGroups(storage, t.defaultGroup);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [target, setTarget] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const runner = useCommandRunner({
    backend,
    encoding: "utf-8",
    invalidHex: t.invalidHex,
    onResult: setError,
    errorMessage,
  });

  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key === "serialbot.preferences") setPrefs(loadPrefs(storage));
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, [storage]);

  useEffect(() => {
    const dark = window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
    applyAppearance(document.documentElement, prefs, dark);
  }, [prefs]);

  const open = sessions.filter((s) => s.state === "open");
  const applySessions = useCallback((list: SessionInfo[]) => setSessions(list), []);
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let gone = false;
    void backend.onSessionsChanged(applySessions).then((u) => (gone ? u() : (unlisten = u)));
    backend.listSessions().then(applySessions, (e: unknown) => setError(errorMessage(e)));
    return () => {
      gone = true;
      unlisten?.();
    };
  }, [backend, applySessions]);

  // Keep the pick while that session is open; otherwise take the first open one.
  const picked = open.find((s) => s.id === target) ?? open[0] ?? null;

  return (
    <div className="command-window">
      <label className="send-to">
        {t.sendTo}
        <select
          aria-label={t.sendTo}
          value={picked?.id ?? ""}
          disabled={open.length === 0}
          onChange={(e) => setTarget(Number(e.target.value))}
        >
          {open.map((s) => (
            <option key={s.id} value={s.id}>
              {s.alias ? `${s.alias} (${s.port})` : s.port}
            </option>
          ))}
        </select>
      </label>
      {open.length === 0 && <p className="muted">{t.noOpenSession}</p>}
      <CommandPanel
        t={t}
        groups={groups}
        onChange={change}
        enabled={picked !== null}
        onSend={(c) => picked && void runner.send(picked.id, c)}
        run={runner.run}
        onRun={(g) => picked && void runner.runGroup(picked.id, g)}
        onStop={runner.stop}
        defaultEnding="crlf"
      />
      {error && (
        <p className="error" role="status">
          {error}
        </p>
      )}
    </div>
  );
}
