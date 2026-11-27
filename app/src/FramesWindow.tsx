// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { useCallback, useEffect, useRef, useState } from "react";
import {
  type Backend,
  type DecodedFrame,
  errorMessage,
  type Protocol,
  type SessionInfo,
} from "./api";
import { browserStorage } from "./commands";
import { type Lang, strings } from "./i18n";
import { applyAppearance, loadPrefs, resolveLanguage } from "./prefs";
import { formatTime } from "./timefmt";

interface Props {
  backend: Backend;
  lang: Lang;
  storage?: Storage | null;
}

/** Frames this window keeps; older ones scroll away. */
export const KEEP_FRAMES = 5000;
/** Rows drawn at once (the newest of the matching frames). */
export const SHOWN_ROWS = 1000;
/** Frames asked for per read. */
const READ_FRAMES = 1000;
/** After data arrives, read once more this much later, for messages the
 * decoder only ends after a quiet gap. */
const SETTLE_MS = 300;
/** Key in local storage for the chosen protocol. */
export const PROTOCOL_KEY = "serialbot.frames.protocol";

function readProtocol(storage: Storage | null): string | null {
  try {
    return storage?.getItem(PROTOCOL_KEY) ?? null;
  } catch {
    return null;
  }
}

function writeProtocol(storage: Storage | null, id: string): void {
  try {
    storage?.setItem(PROTOCOL_KEY, id);
  } catch {
    // Not kept for next time: nothing else changes.
  }
}

const MARKS: Record<DecodedFrame["status"], string> = {
  ok: "✓",
  badChecksum: "✗",
  incomplete: "…",
  unknown: "?",
};

/**
 * Decoded frames in their own window: pick a session (open or closed: its
 * log stays) and a protocol; every message the port sent or received is a
 * row, newest at the bottom. A row opens its fields with the bytes each one
 * takes. Decoding happens in the backend, which reads the whole log, so
 * choosing a protocol later still shows what came before.
 */
export function FramesWindow({ backend, lang, storage = browserStorage() }: Props) {
  const [prefs, setPrefs] = useState(() => loadPrefs(storage));
  const uiLang = resolveLanguage(prefs.language, lang);
  const t = strings(uiLang);
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [target, setTarget] = useState<number | null>(null);
  const [protocols, setProtocols] = useState<Protocol[]>([]);
  const [protocol, setProtocol] = useState<string | null>(() => readProtocol(storage));
  const [frames, setFrames] = useState<DecodedFrame[]>([]);
  const [missed, setMissed] = useState(0);
  const [selected, setSelected] = useState<number | null>(null);
  const [filter, setFilter] = useState("");
  const [error, setError] = useState<string | null>(null);
  const next = useRef(0);
  const reading = useRef(false);
  const again = useRef(false);
  const list = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);

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

  const applySessions = useCallback((all: SessionInfo[]) => setSessions(all), []);
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let gone = false;
    void backend.onSessionsChanged(applySessions).then((u) => (gone ? u() : (unlisten = u)));
    backend.listSessions().then(applySessions, (e: unknown) => setError(errorMessage(e)));
    backend.protocols().then(setProtocols, (e: unknown) => setError(errorMessage(e)));
    return () => {
      gone = true;
      unlisten?.();
    };
  }, [backend, applySessions]);

  // Keep the pick while that session exists; otherwise the first open one.
  const picked =
    sessions.find((s) => s.id === target) ??
    sessions.find((s) => s.state === "open") ??
    sessions[0] ??
    null;
  const chosen = protocols.find((p) => p.id === protocol) ?? protocols[0] ?? null;
  const sessionId = picked?.id ?? null;
  const protocolId = chosen?.id ?? null;
  const frameLang = uiLang === "zh" ? "zh" : "en";

  // Reads every frame after the last one, then again if asked meanwhile.
  const read = useCallback(async () => {
    if (sessionId === null || protocolId === null) return;
    if (reading.current) {
      again.current = true;
      return;
    }
    reading.current = true;
    try {
      let more = true;
      while (more) {
        const chunk = await backend.readFrames(
          sessionId,
          protocolId,
          frameLang,
          next.current,
          READ_FRAMES,
        );
        next.current = chunk.next;
        if (chunk.missed > 0) setMissed((m) => m + chunk.missed);
        if (chunk.frames.length > 0) {
          setFrames((all) => [...all, ...chunk.frames].slice(-KEEP_FRAMES));
        }
        more = chunk.frames.length === READ_FRAMES || again.current;
        again.current = false;
      }
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      reading.current = false;
    }
  }, [backend, sessionId, protocolId, frameLang]);

  // A new session, protocol or language: start from the first frame.
  useEffect(() => {
    next.current = 0;
    atBottom.current = true;
    setFrames([]);
    setMissed(0);
    setSelected(null);
    void read();
  }, [read]);

  // New data: read now, and once more after the decoder's gap.
  useEffect(() => {
    if (sessionId === null) return undefined;
    let settle: ReturnType<typeof setTimeout> | undefined;
    let unlisten: (() => void) | null = null;
    let gone = false;
    void backend
      .onLogChanged((ids) => {
        if (!ids.includes(sessionId)) return;
        void read();
        clearTimeout(settle);
        settle = setTimeout(() => void read(), SETTLE_MS);
      })
      .then((u) => (gone ? u() : (unlisten = u)));
    return () => {
      gone = true;
      clearTimeout(settle);
      unlisten?.();
    };
  }, [backend, sessionId, read]);

  // Follow new rows while the list is scrolled to the bottom.
  useEffect(() => {
    const el = list.current;
    if (el && atBottom.current) el.scrollTop = el.scrollHeight;
  }, [frames]);

  const needle = filter.trim().toLowerCase();
  const matching = needle
    ? frames.filter((f) =>
        `${f.name} ${f.summary} ${f.bytes}`.toLowerCase().includes(needle),
      )
    : frames;
  const shown = matching.slice(-SHOWN_ROWS);
  const bad = frames.filter((f) => f.status !== "ok").length;
  const detail = frames.find((f) => f.seq === selected) ?? null;

  return (
    <div className="frames-window">
      <div className="frames-bar">
        <label>
          {t.framesPort}
          <select
            aria-label={t.framesPort}
            value={picked?.id ?? ""}
            disabled={sessions.length === 0}
            onChange={(e) => setTarget(Number(e.target.value))}
          >
            {sessions.map((s) => (
              <option key={s.id} value={s.id}>
                {s.alias ? `${s.alias} (${s.port})` : s.port}
              </option>
            ))}
          </select>
        </label>
        <label>
          {t.framesProtocol}
          <select
            aria-label={t.framesProtocol}
            value={chosen?.id ?? ""}
            onChange={(e) => {
              setProtocol(e.target.value);
              writeProtocol(storage, e.target.value);
            }}
          >
            {protocols.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </label>
        <input
          className="grow"
          type="search"
          aria-label={t.framesFilter}
          placeholder={t.framesFilter}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <span className="muted">{t.framesCount(frames.length, bad)}</span>
      </div>
      {sessions.length === 0 && <p className="muted">{t.framesNoPort}</p>}
      {sessions.length > 0 && frames.length === 0 && <p className="muted">{t.framesNone}</p>}
      {missed > 0 && <p className="muted">{t.framesMissed(missed)}</p>}
      <div
        className="frames-list"
        ref={list}
        onScroll={(e) => {
          const el = e.currentTarget;
          atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
        }}
      >
        <table className="frames-table">
          <thead>
            <tr>
              <th aria-label={t.frameStatus.ok} />
              <th>{t.frameTime}</th>
              <th>{t.frameDirection}</th>
              <th>{t.frameName}</th>
              <th>{t.frameSummary}</th>
            </tr>
          </thead>
          <tbody>
            {shown.map((f) => (
              <tr
                key={f.seq}
                className={`frame st-${f.status}${f.seq === selected ? " selected" : ""}`}
                aria-selected={f.seq === selected}
                onClick={() => setSelected(f.seq === selected ? null : f.seq)}
              >
                <td className="mark" title={t.frameStatus[f.status]}>
                  {MARKS[f.status]}
                </td>
                <td className="time">{formatTime(f.timestampUs)}</td>
                <td className={`dir ${f.direction}`}>{f.direction.toUpperCase()}</td>
                <td className="name">{f.name}</td>
                <td className="summary">{f.summary}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {detail && (
        <section className="frame-detail" aria-label={detail.name}>
          <p>
            <strong>{detail.name}</strong>{" "}
            <span className={detail.status === "ok" ? "ok" : "error"}>
              {MARKS[detail.status]} {t.frameStatus[detail.status]}
            </span>
          </p>
          <code className="frame-bytes">{detail.bytes}</code>
          {detail.fields.length > 0 && (
            <table className="frame-fields">
              <thead>
                <tr>
                  <th>{t.frameField}</th>
                  <th>{t.frameValue}</th>
                  <th>{t.frameBytes}</th>
                </tr>
              </thead>
              <tbody>
                {detail.fields.map((field) => (
                  <tr key={`${field.offset}-${field.name}`}>
                    <td>{field.name}</td>
                    <td>{field.text}</td>
                    <td className="frame-bytes">
                      {detail.bytes
                        .split(" ")
                        .slice(field.offset, field.offset + field.len)
                        .join(" ")}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>
      )}
      {error && (
        <p className="error" role="status">
          {error}
        </p>
      )}
    </div>
  );
}
