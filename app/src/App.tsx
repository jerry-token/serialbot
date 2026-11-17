import { useCallback, useEffect, useRef, useState } from "react";
import type { Backend, PortInfo, SessionInfo, SessionStateName } from "./api";
import { errorMessage } from "./api";
import { encodeInput } from "./bytes";
import { ConnectionBar, type PortSettings } from "./components/ConnectionBar";
import { LogView } from "./components/LogView";
import { SendBar, type SendSettings } from "./components/SendBar";
import { type Lang, strings } from "./i18n";
import { Terminal } from "./terminal";

/** Payload budget of one log poll. */
export const READ_BYTES = 256 * 1024;

interface Props {
  backend: Backend;
  lang: Lang;
  /** Delay between log polls when nothing new arrived. */
  pollMs?: number;
}

const nowUs = () => Date.now() * 1000;

export function App({ backend, lang, pollMs = 30 }: Props) {
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
  const [session, setSession] = useState<SessionInfo | null>(null);
  const [status, setStatus] = useState<SessionStateName>("closed");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [send, setSend] = useState<SendSettings>({ text: "", hex: false, ending: "crlf" });
  const [showTimestamps, setShowTimestamps] = useState(true);
  const [hexView, setHexView] = useState(false);
  const [showSent, setShowSent] = useState(true);
  const [autoScroll, setAutoScroll] = useState(true);
  const [counts, setCounts] = useState({ rx: 0, tx: 0 });
  const terminal = useRef(new Terminal());
  const [version, setVersion] = useState(0);
  const redraw = () => setVersion((v) => v + 1);

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

  useEffect(() => {
    void refreshPorts();
  }, [refreshPorts]);

  // Poll the session's log while it exists.
  useEffect(() => {
    if (!session) return undefined;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let cursor = 0;
    const tick = async () => {
      try {
        const chunk = await backend.readLog(session.id, cursor, READ_BYTES);
        if (cancelled) return;
        cursor = chunk.next;
        if (chunk.missed > 0) terminal.current.info(t.missed(chunk.missed), nowUs());
        if (chunk.entries.length > 0) {
          terminal.current.push(chunk.entries);
          let rx = 0;
          let tx = 0;
          for (const e of chunk.entries) {
            if (e.kind === "rx") rx += e.data.length;
            if (e.kind === "tx") tx += e.data.length;
          }
          setCounts((c) => ({ rx: c.rx + rx, tx: c.tx + tx }));
        }
        if (chunk.missed > 0 || chunk.entries.length > 0) redraw();
        if (chunk.state !== "open" && chunk.entries.length === 0) {
          // Everything received before the port went away has been shown.
          setStatus(chunk.state);
          terminal.current.info(chunk.error ?? t.deviceGone, nowUs());
          redraw();
          return;
        }
        timer = setTimeout(() => void tick(), chunk.entries.length > 0 ? 0 : pollMs);
      } catch (e) {
        if (!cancelled) setError(errorMessage(e));
      }
    };
    void tick();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [backend, session, pollMs, t]);

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
      setSession(opened);
      setStatus(opened.state);
      setCounts({ rx: 0, tx: 0 });
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const close = async () => {
    if (!session) return;
    setBusy(true);
    try {
      await backend.closeSession(session.id);
      setError(null);
    } catch (e) {
      // The session is gone either way; report why closing failed.
      setError(errorMessage(e));
    } finally {
      setSession(null);
      setStatus("closed");
      setBusy(false);
    }
  };

  const sendData = async () => {
    if (!session) return;
    let data: Uint8Array;
    try {
      data = encodeInput(send.text, send.hex, send.ending);
    } catch {
      setError(t.invalidHex);
      return;
    }
    try {
      await backend.writeData(session.id, data);
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const toggleHexView = (hex: boolean) => {
    setHexView(hex);
    terminal.current.setHex(hex);
    redraw();
  };

  const clear = () => {
    terminal.current.clear();
    setCounts({ rx: 0, tx: 0 });
    redraw();
  };

  const statusText = {
    open: t.statusOpen,
    disconnected: t.statusDisconnected,
    closed: t.statusClosed,
    failed: t.statusFailed,
  }[status];

  return (
    <div className="app">
      <ConnectionBar
        t={t}
        ports={ports}
        settings={settings}
        onChange={setSettings}
        onRefresh={() => void refreshPorts()}
        connected={session !== null}
        busy={busy}
        onOpen={() => void open()}
        onClose={() => void close()}
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
        <button type="button" onClick={clear}>
          {t.clear}
        </button>
      </div>
      <LogView
        lines={terminal.current.view}
        showTimestamps={showTimestamps}
        showSent={showSent}
        autoScroll={autoScroll}
        version={version}
      />
      <SendBar
        t={t}
        settings={send}
        onChange={setSend}
        enabled={status === "open"}
        onSend={() => void sendData()}
      />
      <div className="bar status" role="status">
        <span className={`state ${status}`}>
          {session ? `${session.port} · ` : ""}
          {statusText}
        </span>
        <span>
          {t.rxBytes} {counts.rx} · {t.txBytes} {counts.tx}
        </span>
        {error && <span className="error">{error}</span>}
      </div>
    </div>
  );
}
