import { useCallback, useEffect, useRef, useState } from "react";
import type {
  Backend,
  McpSettings,
  ApprovalRequest,
  McpStatus,
  PortInfo,
  SessionInfo,
  CaptureStatus,
  ExportFormat,
  FileSendStatus,
  RepeatStatus,
  SignalsInfo,
  Unlisten,
} from "./api";
import { errorMessage, formatBytes, saveFileName } from "./api";
import { encodeInput, withEndings } from "./bytes";
import { withChecksum } from "./checksum";
import {
  type Command,
  type CommandGroup,
  browserStorage,
  loadGroups,
  runSequence,
  saveGroups,
} from "./commands";
import { CommandPanel, type RunProgress } from "./components/CommandPanel";
import { DisplaySettings } from "./components/DisplaySettings";
import { FileSend, type FileSendOptions } from "./components/FileSend";
import { endpointPath, isNetworkPath } from "./endpoint";
import { SearchBar } from "./components/SearchBar";
import { ConnectionBar, type PortSettings } from "./components/ConnectionBar";
import { LogView } from "./components/LogView";
import { McpPanel } from "./components/McpPanel";
import { ApprovalDialog } from "./components/ApprovalDialog";
import { SaveMenu } from "./components/SaveMenu";
import { SendBar, type SendSettings } from "./components/SendBar";
import { BREAK_MS, SignalBar } from "./components/SignalBar";
import type { SignalStep } from "./signals";
import { SessionTabs, Toggle } from "./components/SessionTabs";
import { type Page, Sidebar } from "./components/Sidebar";
import { ClearIcon, ListIcon, SearchIcon } from "./components/icons";
import { type Lang, strings } from "./i18n";
import { type Line, Terminal } from "./terminal";

/** Storage key of the DTR/RTS sequence. */
const SEQUENCE_KEY = "serialbot.signalSequence";

/** Reads a stored string; storage may be missing or refuse access. */
function readItem(storage: Storage | null, key: string): string | null {
  try {
    return storage?.getItem(key) ?? null;
  } catch {
    return null;
  }
}

/** Stores a string if storage allows it. */
function writeItem(storage: Storage | null, key: string, value: string): void {
  try {
    storage?.setItem(key, value);
  } catch {
    // Private mode or a full quota: the value just is not kept.
  }
}

/** Payload budget of one log read. */
export const READ_BYTES = 256 * 1024;

/** Sent texts remembered for ↑ / ↓. */
export const HISTORY_SIZE = 50;

/** How often the progress of running timed sends is refreshed. */
export const REPEAT_POLL_MS = 500;

/** How often the size of running captures is refreshed. */
export const CAPTURE_POLL_MS = 1000;

interface Props {
  backend: Backend;
  lang: Lang;
  /** Clipboard writer (replaceable in tests). */
  copy?: (text: string) => Promise<void>;
  /** Where saved commands are kept (replaceable in tests). */
  storage?: Storage | null;
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

export function App({ backend, lang, copy = defaultCopy, storage = browserStorage() }: Props) {
  const t = strings(lang);
  const [groups, setGroups] = useState<CommandGroup[]>(() =>
    loadGroups(storage, t.defaultGroup),
  );
  const [showCommands, setShowCommands] = useState(false);
  const [run, setRun] = useState<RunProgress | null>(null);
  const runStop = useRef<AbortController | null>(null);
  const [ports, setPorts] = useState<PortInfo[]>([]);
  const [settings, setSettings] = useState<PortSettings>({
    kind: "serial",
    host: "",
    netPort: "",
    localPort: "",
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
  const [send, setSend] = useState<SendSettings>({
    text: "",
    hex: false,
    ending: "crlf",
    checksum: "none",
    repeat: false,
    intervalMs: "1000",
  });
  const [history, setHistory] = useState<string[]>([]);
  const [repeats, setRepeats] = useState<Record<number, RepeatStatus>>({});
  const [captures, setCaptures] = useState<Record<number, CaptureStatus>>({});
  const [fileSends, setFileSends] = useState<Record<number, FileSendStatus>>({});
  const [showTimestamps, setShowTimestamps] = useState(true);
  const [hexView, setHexView] = useState(false);
  const [showSent, setShowSent] = useState(true);
  const [showDelta, setShowDelta] = useState(false);
  const [filterMatches, setFilterMatches] = useState(false);
  const [autoScroll, setAutoScroll] = useState(true);
  const [mcp, setMcp] = useState<McpStatus | null>(null);
  const [approvals, setApprovals] = useState<ApprovalRequest[]>([]);
  const [page, setPage] = useState<Page>("terminal");
  const [signals, setSignals] = useState<Record<number, SignalsInfo>>({});
  const storeSignals = useCallback(
    (info: SignalsInfo) => setSignals((all) => ({ ...all, [info.id]: info })),
    [],
  );
  const views = useRef(new Map<number, SessionView>());
  const [encoding, setEncoding] = useState("utf-8");
  const [frameGapMs, setFrameGapMs] = useState(0);
  // Display settings also apply to sessions that appear later.
  const display = useRef({ hex: false, encoding: "utf-8", frameGapMs: 0, ansi: true });
  const [ansi, setAnsi] = useState(true);
  const [frozen, setFrozen] = useState<{ id: number; lines: readonly Line[] } | null>(null);
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState("");
  /** Index into the matches; null follows the newest match. */
  const [matchPos, setMatchPos] = useState<number | null>(null);
  const [version, setVersion] = useState(0);
  const redraw = () => setVersion((v) => v + 1);

  const viewOf = (id: number): SessionView => {
    let view = views.current.get(id);
    if (!view) {
      const terminal = new Terminal();
      terminal.setHex(display.current.hex);
      terminal.setEncoding(display.current.encoding);
      terminal.setFrameGap(display.current.frameGapMs);
      terminal.setAnsi(display.current.ansi);
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
          // After a reconnect, a later disconnect is reported again.
          if (chunk.state === "open") view.ended = false;
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

  // Keeps the selected port while it is still present.
  const applyPorts = useCallback((list: PortInfo[]) => {
    setPorts(list);
    setSettings((s) =>
      list.some((p) => p.path === s.port) ? s : { ...s, port: list[0]?.path ?? "" },
    );
  }, []);

  const refreshPorts = useCallback(async () => {
    try {
      applyPorts(await backend.listPorts());
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }, [backend, applyPorts]);

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
        keep(await backend.onSignalsChanged(storeSignals));
        keep(await backend.onPortsChanged(applyPorts));
        keep(await backend.onApprovalsChanged(setApprovals));
        setApprovals(await backend.pendingApprovals());
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
  }, [backend, applySessions, applyPorts, pull, refreshPorts, storeSignals]);

  useEffect(() => {
    saveGroups(storage, groups);
  }, [storage, groups]);
  // The DTR/RTS sequence last typed, kept across restarts.
  const [sequence, setSequence] = useState(() => readItem(storage, SEQUENCE_KEY) ?? "");
  useEffect(() => writeItem(storage, SEQUENCE_KEY, sequence), [storage, sequence]);

  // A run does not outlive the window.
  useEffect(() => () => runStop.current?.abort(), []);

  const active = sessions.find((s) => s.id === activeId) ?? null;
  const activeOpen = active?.state === "open";

  // Show the line levels of the selected session right away; changes then
  // arrive as events.
  useEffect(() => {
    if (activeId === null || !activeOpen) return;
    backend.signals(activeId).then(storeSignals, (e: unknown) => setError(errorMessage(e)));
  }, [backend, activeId, activeOpen, storeSignals]);

  const setLines = (levels: { dtr?: boolean; rts?: boolean }) => {
    if (!active) return;
    backend
      .setSignals(active.id, levels)
      .then(storeSignals, (e: unknown) => setError(errorMessage(e)));
  };
  const runSignalSequence = (steps: SignalStep[]) => {
    if (!active) return;
    backend
      .runSignals(active.id, steps)
      .then(storeSignals, (e: unknown) => setError(errorMessage(e)));
  };
  const sendBreak = () => {
    if (!active) return;
    backend.sendBreak(active.id, BREAK_MS).catch((e: unknown) => setError(errorMessage(e)));
  };
  const activeView = activeId !== null ? views.current.get(activeId) : undefined;
  // What the bar would open: a serial port or a network endpoint URL.
  const target =
    settings.kind === "serial"
      ? { path: settings.port }
      : endpointPath(settings.kind, {
          host: settings.host,
          port: settings.netPort,
          localPort: settings.localPort,
        });
  const portSession =
    ("path" in target && sessions.find((s) => s.port === target.path)) || null;

  const open = async () => {
    if ("error" in target) {
      setError(t.endpointErrors[target.error]);
      return;
    }
    // Network endpoints ignore the line settings.
    const baudRate = settings.kind === "serial" ? Number(settings.baudRate) : 115200;
    if (!Number.isInteger(baudRate) || baudRate <= 0) {
      setError(t.invalidBaud);
      return;
    }
    setBusy(true);
    try {
      const opened = await backend.openSession({
        port: target.path,
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
      if (send.hex) data = withChecksum(send.checksum, data);
    } catch {
      setError(t.invalidHex);
      return;
    }
    const interval = Number(send.intervalMs);
    if (send.repeat && !(Number.isInteger(interval) && interval >= 1 && interval <= 86_400_000)) {
      setError(t.invalidInterval);
      return;
    }
    try {
      if (!send.hex && encoding !== "utf-8") {
        data = Uint8Array.from(
          await backend.encodeText(withEndings(send.text, send.ending), encoding),
        );
      }
      if (send.repeat) {
        const status = await backend.startRepeat(active.id, data, interval, null);
        setRepeats((all) => ({ ...all, [active.id]: status }));
      } else {
        await backend.writeData(active.id, data);
      }
      setError(null);
      // Remember what was sent; the newest entry is last, without repeats.
      const text = send.text;
      setHistory((h) => [...h.filter((x) => x !== text), text].slice(-HISTORY_SIZE));
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  // Saved commands are sent like the send box, with their own settings.
  const commandBytes = async (c: Command): Promise<Uint8Array> => {
    if (c.hex) {
      try {
        return encodeInput(c.text, true, "none");
      } catch {
        throw t.invalidHex;
      }
    }
    if (encoding !== "utf-8") {
      return Uint8Array.from(await backend.encodeText(withEndings(c.text, c.ending), encoding));
    }
    return encodeInput(c.text, false, c.ending);
  };

  const sendCommand = async (c: Command) => {
    if (!active) return;
    try {
      await backend.writeData(active.id, await commandBytes(c));
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  // Runs a group on the session selected when it starts.
  const runGroup = async (group: CommandGroup) => {
    if (!active || runStop.current) return;
    const id = active.id;
    const stop = new AbortController();
    runStop.current = stop;
    const total = group.commands.length;
    try {
      await runSequence(
        group.commands,
        async (c) => {
          await backend.writeData(id, await commandBytes(c));
        },
        stop.signal,
        (i) => setRun({ groupId: group.id, index: i + 1, total }),
      );
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      runStop.current = null;
      setRun(null);
    }
  };

  const stopRepeat = async () => {
    if (!active) return;
    try {
      const status = await backend.stopRepeat(active.id);
      setRepeats((all) => ({ ...all, [active.id]: status }));
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  // While timed sends run, refresh their progress twice a second.
  const runningIds = Object.entries(repeats)
    .filter(([, r]) => r.running)
    .map(([id]) => Number(id))
    .join(",");
  useEffect(() => {
    if (runningIds === "") return undefined;
    const timer = setInterval(() => {
      for (const id of runningIds.split(",").map(Number)) {
        backend.repeatStatus(id).then(
          (status) => {
            setRepeats((all) => ({ ...all, [id]: status }));
            if (status.error) setError(status.error);
          },
          (e: unknown) => setError(errorMessage(e)),
        );
      }
    }, REPEAT_POLL_MS);
    return () => clearInterval(timer);
  }, [backend, runningIds]);

  // While files are being sent, refresh their progress like timed sends.
  const sendingIds = Object.entries(fileSends)
    .filter(([, f]) => f.running)
    .map(([id]) => Number(id))
    .join(",");
  useEffect(() => {
    if (sendingIds === "") return undefined;
    const timer = setInterval(() => {
      for (const id of sendingIds.split(",").map(Number)) {
        backend.fileSendStatus(id).then(
          (status) => {
            setFileSends((all) => ({ ...all, [id]: status }));
            if (status.error) setError(status.error);
          },
          (e: unknown) => setError(errorMessage(e)),
        );
      }
    }, REPEAT_POLL_MS);
    return () => clearInterval(timer);
  }, [backend, sendingIds]);

  const startFileSend = async ({ chunk, delayMs }: FileSendOptions) => {
    if (!active) return;
    const id = active.id;
    try {
      const path = await backend.chooseOpenFile();
      if (!path) return;
      const status = await backend.sendFile(id, path, chunk, delayMs);
      setFileSends((all) => ({ ...all, [id]: status }));
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const stopFileSend = async (id: number) => {
    try {
      const status = await backend.stopFileSend(id);
      setFileSends((all) => ({ ...all, [id]: status }));
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const exportLog = async (format: ExportFormat): Promise<string | null> => {
    if (!active) return null;
    const path = await backend.chooseSaveFile(saveFileName(active.port, format), format);
    if (!path) return null;
    const { bytes } = await backend.exportLog(active.id, path, format);
    return t.saved(formatBytes(bytes), path);
  };

  const startCapture = async (format: ExportFormat): Promise<string | null> => {
    if (!active) return null;
    const id = active.id;
    const path = await backend.chooseSaveFile(saveFileName(active.port, format), format);
    if (!path) return null;
    const status = await backend.startCapture(id, path, format);
    setCaptures((all) => ({ ...all, [id]: status }));
    return null;
  };

  const stopCapture = async (): Promise<string | null> => {
    if (!active) return null;
    const id = active.id;
    const status = await backend.stopCapture(id);
    setCaptures((all) => ({ ...all, [id]: status }));
    return status.path ? t.saved(formatBytes(status.bytes), status.path) : null;
  };

  // While captures run, refresh their size (and notice when they end).
  const capturingIds = Object.entries(captures)
    .filter(([, c]) => c.active)
    .map(([id]) => Number(id))
    .join(",");
  useEffect(() => {
    if (capturingIds === "") return undefined;
    const timer = setInterval(() => {
      for (const id of capturingIds.split(",").map(Number)) {
        backend.captureStatus(id).then(
          (status) => {
            setCaptures((all) => ({ ...all, [id]: status }));
            if (status.error) setError(status.error);
          },
          (e: unknown) => setError(errorMessage(e)),
        );
      }
    }, CAPTURE_POLL_MS);
    return () => clearInterval(timer);
  }, [backend, capturingIds]);

  const toggleHexView = (hex: boolean) => {
    display.current.hex = hex;
    setHexView(hex);
    for (const view of views.current.values()) view.terminal.setHex(hex);
    redraw();
  };

  const changeEncoding = (label: string) => {
    display.current.encoding = label;
    setEncoding(label);
    for (const view of views.current.values()) view.terminal.setEncoding(label);
    redraw();
  };

  const changeFrameGap = (ms: number) => {
    display.current.frameGapMs = ms;
    setFrameGapMs(ms);
    for (const view of views.current.values()) view.terminal.setFrameGap(ms);
    redraw();
  };

  const changeAnsi = (on: boolean) => {
    display.current.ansi = on;
    setAnsi(on);
    for (const view of views.current.values()) view.terminal.setAnsi(on);
    redraw();
  };

  const togglePause = (pause: boolean) => {
    setFrozen(pause && activeView && activeId !== null
      ? { id: activeId, lines: activeView.terminal.view.slice() }
      : null);
  };

  // Ctrl/Cmd+F opens the search on the terminal page.
  useEffect(() => {
    if (page !== "terminal") return undefined;
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
        e.preventDefault();
        setSearchOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [page]);

  const live = activeView?.terminal.view ?? [];
  const paused = frozen !== null && frozen.id === activeId;
  const shown = paused ? frozen.lines : live;
  const unfiltered = showSent ? shown : shown.filter((l) => l.kind !== "tx");
  const lastFrozenId = paused ? (frozen.lines.at(-1)?.id ?? -1) : -1;
  const newWhilePaused = paused ? live.filter((l) => l.id > lastFrozenId).length : 0;
  const needle = searchOpen ? query.toLowerCase() : "";
  const matching = needle ? unfiltered.filter((l) => l.text.toLowerCase().includes(needle)) : [];
  const matchIds = matching.map((l) => l.id);
  // With the filter on, only matching lines are shown (all of them while the query is empty).
  const visibleLines = needle && filterMatches ? matching : unfiltered;
  const matchIndex =
    matchIds.length === 0 ? -1 : Math.min(matchPos ?? matchIds.length - 1, matchIds.length - 1);
  const currentMatch = matchIndex >= 0 ? (matchIds[matchIndex] ?? null) : null;

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

  // Changes arrive as sessions-changed events.
  const sessionCall = async (call: (id: number) => Promise<void>) => {
    if (activeId === null) return;
    try {
      await call(activeId);
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const fileSend = active ? fileSends[active.id] : undefined;

  const statusText = active
    ? {
        open: t.statusOpen,
        disconnected: t.statusDisconnected,
        closed: t.statusClosed,
        failed: t.statusFailed,
      }[active.state]
    : t.statusClosed;

  const mcpPort = mcp?.running && mcp.url ? Number(new URL(mcp.url).port) : null;

  return (
    <div className="shell">
      <Sidebar t={t} page={page} onSelect={setPage} mcpRunning={mcpPort !== null} />
      <main className="main">
        {page === "terminal" ? (
          <>
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
            <SessionTabs
              t={t}
              sessions={sessions}
              activeId={activeId}
              onSelect={setActiveId}
              onClose={(id) => void close(id)}
            >
              <Toggle
                label={t.showTimestamps}
                hint={t.showTimestampsHint}
                pressed={showTimestamps}
                onChange={setShowTimestamps}
              />
              <Toggle
                label={t.showDelta}
                hint={t.showDeltaHint}
                pressed={showDelta}
                onChange={setShowDelta}
              />
              <Toggle
                label={t.hexView}
                hint={t.hexViewHint}
                pressed={hexView}
                onChange={toggleHexView}
              />
              <Toggle
                label={t.showSent}
                hint={t.showSentHint}
                pressed={showSent}
                onChange={setShowSent}
              />
              <Toggle
                label={t.autoScroll}
                hint={t.autoScrollHint}
                pressed={autoScroll}
                onChange={setAutoScroll}
              />
              <Toggle
                label={paused ? t.paused(newWhilePaused) : t.pause}
                hint={t.pauseHint}
                pressed={paused}
                onChange={togglePause}
              />
              <button
                type="button"
                className="icon-button"
                aria-label={t.commands}
                title={t.commandsHint}
                aria-pressed={showCommands}
                onClick={() => setShowCommands((o) => !o)}
              >
                <ListIcon size={16} />
              </button>
              <button
                type="button"
                className="icon-button"
                aria-label={t.search}
                title={`${t.search} (Ctrl+F)`}
                aria-pressed={searchOpen}
                onClick={() => setSearchOpen((o) => !o)}
              >
                <SearchIcon size={16} />
              </button>
              <SaveMenu
                t={t}
                enabled={active !== null}
                capture={active ? (captures[active.id] ?? null) : null}
                onExport={exportLog}
                onStartCapture={startCapture}
                onStopCapture={stopCapture}
              />
              <DisplaySettings
                t={t}
                encoding={encoding}
                onEncoding={changeEncoding}
                frameGapMs={frameGapMs}
                onFrameGap={changeFrameGap}
                ansi={ansi}
                onAnsi={changeAnsi}
              />
              <button
                type="button"
                className="icon-button"
                aria-label={t.clear}
                title={t.clear}
                onClick={clear}
                disabled={!activeView}
              >
                <ClearIcon size={16} />
              </button>
            </SessionTabs>
            <div className="work">
              {active ? (
                <div className="log-area">
                  {searchOpen && (
                    <SearchBar
                      t={t}
                      query={query}
                      onQuery={(q) => {
                        setQuery(q);
                        setMatchPos(null);
                      }}
                      filter={filterMatches}
                      onFilter={setFilterMatches}
                      current={matchIndex + 1}
                      total={matchIds.length}
                      onStep={(delta) =>
                        setMatchPos(
                          (matchIndex + delta + matchIds.length) % matchIds.length,
                        )
                      }
                      onClose={() => {
                        setSearchOpen(false);
                        setMatchPos(null);
                      }}
                    />
                  )}
                  <LogView
                    key={activeId ?? 0}
                    lines={visibleLines}
                    showTimestamps={showTimestamps}
                    showDelta={showDelta}
                    autoScroll={autoScroll && !paused}
                    version={version}
                    matches={new Set(matchIds)}
                    currentMatch={currentMatch}
                  />
                </div>
              ) : (
                <div className="log empty" role="log">
                  <p>{t.noSession}</p>
                </div>
              )}
              {showCommands && (
                <CommandPanel
                  t={t}
                  groups={groups}
                  onChange={setGroups}
                  enabled={activeOpen}
                  onSend={(c) => void sendCommand(c)}
                  run={run}
                  onRun={(g) => void runGroup(g)}
                  onStop={() => runStop.current?.abort()}
                  defaultEnding={send.ending}
                />
              )}
            </div>
            <SendBar
              t={t}
              settings={send}
              onChange={setSend}
              enabled={active?.state === "open"}
              onSend={() => void sendData()}
              repeat={active ? (repeats[active.id] ?? null) : null}
              onStopRepeat={() => void stopRepeat()}
              history={history}
              extra={
                <FileSend
                  t={t}
                  enabled={activeOpen && !fileSend?.running}
                  onStart={(options) => void startFileSend(options)}
                />
              }
            />
          </>
        ) : (
          <div className="page">
            {mcp && (
              <McpPanel
                t={t}
                status={mcp}
                onApply={(s: McpSettings) => void mcpCall(() => backend.mcpApply(s))}
                onRegenerate={() => void mcpCall(() => backend.mcpRegenerateToken())}
                copy={copy}
              />
            )}
            <p className="muted">{t.aiAssistantSoon}</p>
          </div>
        )}
        <div className="statusbar" role="status">
          <span className={`state ${active?.state ?? "closed"}`}>
            {active ? `${active.port} · ` : ""}
            {statusText}
          </span>
          {active && (active.state === "disconnected" || active.state === "failed") && (
            <button
              type="button"
              className="chip"
              onClick={() => void sessionCall((id) => backend.reconnectSession(id))}
            >
              {t.reconnect}
            </button>
          )}
          {active && active.state !== "closed" && (
            <button
              type="button"
              className="chip"
              aria-pressed={active.autoReconnect}
              title={t.autoReconnectHint}
              onClick={() =>
                void sessionCall((id) => backend.setAutoReconnect(id, !active.autoReconnect))
              }
            >
              {t.autoReconnect}
            </button>
          )}
          {active && active.state !== "closed" && (
            <button
              type="button"
              className="chip"
              aria-pressed={active.aiAccess}
              title={t.aiAccessHint}
              onClick={() =>
                void sessionCall((id) => backend.setAiAccess(id, !active.aiAccess))
              }
            >
              {t.aiAccess}
            </button>
          )}
          <span>
            {t.rxBytes} {activeView?.rx ?? 0} · {t.txBytes} {activeView?.tx ?? 0}
          </span>
          {active && !isNetworkPath(active.port) && (
            <SignalBar
              t={t}
              signals={signals[active.id] ?? null}
              enabled={activeOpen}
              onSet={setLines}
              onBreak={sendBreak}
              sequence={sequence}
              onSequenceChange={setSequence}
              onRunSequence={runSignalSequence}
            />
          )}
          {active && captures[active.id]?.active && (
            <span className="capturing">
              ●{" "}
              {t.capturing(
                (captures[active.id]?.path ?? "").split(/[\\/]/).pop() ?? "",
                formatBytes(captures[active.id]?.bytes ?? 0),
              )}
            </span>
          )}
          {active && fileSend?.running && (
            <span className="progress">
              {t.sendingFile(
                fileSend.name ?? "",
                fileSend.total > 0 ? Math.floor((fileSend.sent * 100) / fileSend.total) : 0,
                formatBytes(fileSend.sent),
                formatBytes(fileSend.total),
              )}{" "}
              <button type="button" className="chip" onClick={() => void stopFileSend(active.id)}>
                {t.stopFile}
              </button>
            </span>
          )}
          {error && <span className="error">{error}</span>}
          <span className="spacer" />
          <button type="button" className="link" onClick={() => setPage("ai")}>
            <span className={`dot ${mcpPort !== null ? "open" : "closed"}`} aria-hidden="true" />
            {t.mcpShort(mcpPort)}
          </button>
        </div>
      </main>
      <ApprovalDialog
        t={t}
        queue={approvals}
        onAnswer={(id, decision) => {
          setApprovals((queue) => queue.filter((r) => r.id !== id));
          backend.answerApproval(id, decision).catch((e: unknown) => setError(errorMessage(e)));
        }}
      />
    </div>
  );
}
