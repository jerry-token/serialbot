import { useCallback, useEffect, useRef, useState } from "react";
import type { Backend, McpSettings, McpStatus, PortInfo, SessionInfo, Unlisten } from "./api";
import { errorMessage } from "./api";
import { encodeInput } from "./bytes";
import { ConnectionBar, type PortSettings } from "./components/ConnectionBar";
import { LogView } from "./components/LogView";
import { McpPanel } from "./components/McpPanel";
import { SendBar, type SendSettings } from "./components/SendBar";
import { SessionTabs } from "./components/SessionTabs";
import { type Lang, strings } from "./i18n";
import { Terminal } from "./terminal";

/** Payload budget of one log read. */
export const READ_BYTES = 256 * 1024;

interface Props {
  backend: Backend;
  lang: Lang;
  /** Clipboard writer (replaceable in tests). */
  copy?: (text: string) => Promise<void>;
}

/** What the window keeps per session: its rendered log and read position. */
interface SessionView {
  terminal: Terminal;
  cursor: number;
  rx: number;
  tx: number;
  reading: boolean;
  /** A change arrived while reading: read again afterwards. */
  again: boolean;
  /** The end of the session was reported. */
  ended: boolean;
}

const nowUs = () => Date.now() * 1000;

const defaultCopy = (text: string) => navigator.clipboard.writeText(text);

export function App({ backend, lang, copy = defaultCopy }: Props) {
  const t = strings(lang);
  const [ports, setPorts] = useState<PortInfo[]>([]);
  const [settings, setSettings] = useState<PortSettings>({
    port: "",
    baudRate: "115200",
    dataBits: 8,
    parity: "none",
    stopBits: 1,
    flowControl: "none",
  });
  const [sessions, setSessions] = useState<SessionInfo[]>([]);
  const [activeId, setActiveId] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [send, setSend] = useState<SendSettings>({ text: "", hex: false, ending: "crlf" });
  const [showTimestamps, setShowTimestamps] = useState(true);
  const [hexView, setHexView] = useState(false);
  const [showSent, setShowSent] = useState(true);
  const [autoScroll, setAutoScroll] = useState(true);
  const [mcp, setMcp] = useState<McpStatus | null>(null);
  const [mcpOpen, setMcpOpen] = useState(false);
  const views = useRef(new Map<number, SessionView>());
  const hexRef = useRef(false);
  const [version, setVersion] = useState(0);
  const redraw = () => setVersion((v) => v + 1);

  const viewOf = (id: number): SessionView => {
    let view = views.current.get(id);
    if (!view) {
      const terminal = new Terminal();
      terminal.setHex(hexRef.current);
      view = { terminal, cursor: 0, rx: 0, tx: 0, reading: false, again: false, ended: false };
      views.current.set(id, view);
    }
    return view;
  };

  /** Reads everything new of a session; overlapping calls are merged. */
  const pull = useCallback(
    async (id: number) => {
      const view = viewOf(id);
      if (view.reading) {
        view.again = true;
        return;
      }
      view.reading = true;
      try {
        for (;;) {
          view.again = false;
          const chunk = await backend.readLog(id, view.cursor, READ_BYTES);
          if (views.current.get(id) !== view) return; // Closed meanwhile.
          view.cursor = chunk.next;
          if (chunk.missed > 0) view.terminal.info(t.missed(chunk.missed), nowUs());
          view.terminal.push(chunk.entries);
          for (const e of chunk.entries) {
            if (e.kind === "rx") view.rx += e.data.length;
            if (e.kind === "tx") view.tx += e.data.length;
          }
          if (chunk.missed > 0 || chunk.entries.length > 0) redraw();
          if (chunk.entries.length > 0) continue;
          if (chunk.state !== "open" && !view.ended) {
            // Everything received before the port went away has been shown.
            view.ended = true;
            view.terminal.info(chunk.error ?? t.deviceGone, nowUs());
            redraw();
          }
          if (!view.again) break;
        }
      } catch (e) {
        setError(errorMessage(e));
      } finally {
        view.reading = false;
      }
    },
    // viewOf and redraw only touch refs and state setters.
    [backend, t],
  );

  const applySessions = useCallback(
    (list: SessionInfo[]) => {
      setSessions(list);
      const ids = new Set(list.map((s) => s.id));
      for (const id of views.current.keys()) {
        if (!ids.has(id)) views.current.delete(id);
      }
      for (const s of list) {
        if (!views.current.has(s.id)) void pull(s.id);
      }
      setActiveId((active) =>
        active !== null && ids.has(active) ? active : (list.at(-1)?.id ?? null),
      );
    },
    [pull],
  );

  const refreshPorts = useCallback(async () => {
    try {
      const list = await backend.listPorts();
      setPorts(list);
      setSettings((s) =>
        list.some((p) => p.path === s.port) ? s : { ...s, port: list[0]?.path ?? "" },
      );
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [backend]);

  // Subscribe to backend changes, then load the current state.
  useEffect(() => {
    let disposed = false;
    const unlisteners: Unlisten[] = [];
    const keep = (u: Unlisten) => (disposed ? u() : unlisteners.push(u));
    const start = async () => {
      try {
        keep(await backend.onSessionsChanged(applySessions));
        keep(
          await backend.onLogChanged((ids) => {
            for (const id of ids) void pull(id);
          }),
        );
        const list = await backend.listSessions();
        if (!disposed) applySessions(list);
        const status = await backend.mcpStatus();
        if (!disposed) setMcp(status);
      } catch (e) {
        if (!disposed) setError(errorMessage(e));
      }
    };
    void start();
    void refreshPorts();
    return () => {
      disposed = true;
      for (const u of unlisteners) u();
    };
  }, [backend, applySessions, pull, refreshPorts]);

  const active = sessions.find((s) => s.id === activeId) ?? null;
  const activeView = activeId !== null ? views.current.get(activeId) : undefined;
  const portSession = sessions.find((s) => s.port === settings.port) ?? null;

  const open = async () => {
    const baudRate = Number(settings.baudRate);
    if (!Number.isInteger(baudRate) || baudRate <= 0) {
      setError(t.invalidBaud);
      return;
    }
    setBusy(true);
    try {
      const opened = await backend.openSession({
        port: settings.port,
        baudRate,
        dataBits: settings.dataBits,
        parity: settings.parity,
        stopBits: settings.stopBits,
        flowControl: settings.flowControl,
      });
      setSessions((list) => (list.some((s) => s.id === opened.id) ? list : [...list, opened]));
      setActiveId(opened.id);
      void pull(opened.id);
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const close = async (id: number) => {
    setBusy(true);
    try {
      await backend.closeSession(id);
      setError(null);
    } catch (e) {
      // The session is gone either way; report why closing failed.
      setError(errorMessage(e));
    } finally {
      applySessions(sessions.filter((s) => s.id !== id));
      setBusy(false);
    }
  };

  const sendData = async () => {
    if (!active) return;
    let data: Uint8Array;
    try {
      data = encodeInput(send.text, send.hex, send.ending);
    } catch {
      setError(t.invalidHex);
      return;
    }
    try {
      await backend.writeData(active.id, data);
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const toggleHexView = (hex: boolean) => {
    hexRef.current = hex;
    setHexView(hex);
    for (const view of views.current.values()) view.terminal.setHex(hex);
    redraw();
  };

  const clear = () => {
    if (!activeView) return;
    activeView.terminal.clear();
    activeView.rx = 0;
    activeView.tx = 0;
    redraw();
  };

  const mcpCall = async (call: () => Promise<McpStatus>) => {
    try {
      setMcp(await call());
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const statusText = active
    ? {
        open: t.statusOpen,
        disconnected: t.statusDisconnected,
        closed: t.statusClosed,
        failed: t.statusFailed,
      }[active.state]
    : t.statusClosed;

  return (
    <div className="app">
      <ConnectionBar
        t={t}
        ports={ports}
        settings={settings}
        onChange={setSettings}
        onRefresh={() => void refreshPorts()}
        connected={portSession !== null}
        busy={busy}
        onOpen={() => void open()}
        onClose={() => portSession && void close(portSession.id)}
      />
      <div className="bar options">
        <label>
          <input
            type="checkbox"
            checked={showTimestamps}
            onChange={(e) => setShowTimestamps(e.target.checked)}
          />
          {t.showTimestamps}
        </label>
        <label>
          <input
            type="checkbox"
            checked={hexView}
            onChange={(e) => toggleHexView(e.target.checked)}
          />
          {t.hexView}
        </label>
        <label>
          <input
            type="checkbox"
            checked={showSent}
            onChange={(e) => setShowSent(e.target.checked)}
          />
          {t.showSent}
        </label>
        <label>
          <input
            type="checkbox"
            checked={autoScroll}
            onChange={(e) => setAutoScroll(e.target.checked)}
          />
          {t.autoScroll}
        </label>
        <button type="button" onClick={clear} disabled={!activeView}>
          {t.clear}
        </button>
        <span className="spacer" />
        <button
          type="button"
          className="mcp-toggle"
          aria-expanded={mcpOpen}
          onClick={() => setMcpOpen((o) => !o)}
        >
          <span className={`dot ${mcp?.running ? "open" : "closed"}`} aria-hidden="true" />
          {t.mcp}
        </button>
      </div>
      {mcpOpen && mcp && (
        <McpPanel
          t={t}
          status={mcp}
          onApply={(s: McpSettings) => void mcpCall(() => backend.mcpApply(s))}
          onRegenerate={() => void mcpCall(() => backend.mcpRegenerateToken())}
          copy={copy}
        />
      )}
      <SessionTabs
        t={t}
        sessions={sessions}
        activeId={activeId}
        onSelect={setActiveId}
        onClose={(id) => void close(id)}
      />
      <LogView
        key={activeId ?? 0}
        lines={activeView?.terminal.view ?? []}
        showTimestamps={showTimestamps}
        showSent={showSent}
        autoScroll={autoScroll}
        version={version}
      />
      <SendBar
        t={t}
        settings={send}
        onChange={setSend}
        enabled={active?.state === "open"}
        onSend={() => void sendData()}
      />
      <div className="bar status" role="status">
        <span className={`state ${active?.state ?? "closed"}`}>
          {active ? `${active.port} · ` : ""}
          {statusText}
        </span>
        <span>
          {t.rxBytes} {activeView?.rx ?? 0} · {t.txBytes} {activeView?.tx ?? 0}
        </span>
        {error && <span className="error">{error}</span>}
      </div>
    </div>
  );
}
