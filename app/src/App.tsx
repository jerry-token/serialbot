// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
  SequenceStatus,
  SignalsInfo,
  Unlisten,
} from "./api";
import { errorMessage, formatBytes, saveFileName } from "./api";
import { encodeInput, withEndings } from "./bytes";
import { withChecksum } from "./checksum";
import { useCommandRunner } from "./useCommandRunner";
import { useSharedGroups } from "./useSharedGroups";
import { makeHighlighter, parseHighlights } from "./highlight";
import { type Command, type CommandGroup, browserStorage } from "./commands";
import { CommandPanel } from "./components/CommandPanel";
import { SequencePanel } from "./components/SequencePanel";
import { loadSequence, saveSequence, sequenceReport } from "./sequence";
import { DisplaySettings } from "./components/DisplaySettings";
import { FileSend, type FileSendOptions } from "./components/FileSend";
import { HelpPanel } from "./components/HelpPanel";
import { endpointFields, endpointPath, isNetworkPath } from "./endpoint";
import { SearchBar } from "./components/SearchBar";
import { ConnectionBar, type PortSettings } from "./components/ConnectionBar";
import { LogView } from "./components/LogView";
import { McpPanel } from "./components/McpPanel";
import { ApprovalDialog } from "./components/ApprovalDialog";
import { Bookmarks } from "./components/Bookmarks";
import { TtyView } from "./components/TtyView";
import { PlotView } from "./components/PlotView";
import { aiPrompt } from "./aiprompt";
import { type ChartFactory, type ChartTheme, chartThemeFromCss, loadUplotChart } from "./chart";
import { PlotData } from "./plot";
import { loadXtermScreen, type ScreenFactory, type ScreenTheme, themeFromCss } from "./screen";
import { SaveMenu } from "./components/SaveMenu";
import { SendBar, type SendSettings } from "./components/SendBar";
import { BREAK_MS, SignalBar } from "./components/SignalBar";
import type { SignalStep } from "./signals";
import { SessionTabs, Toggle, tabLabel } from "./components/SessionTabs";
import { type Page, Sidebar } from "./components/Sidebar";
import {
  ClearIcon,
  FramesIcon,
  ListIcon,
  RepeatIcon,
  SearchIcon,
} from "./components/icons";
import { type Lang, strings } from "./i18n";
import { DEFAULT_LIMITS, type Line, Terminal } from "./terminal";
import { applyAppearance, loadPrefs, resolveLanguage, savePrefs } from "./prefs";
import { SettingsPanel } from "./components/SettingsPanel";

/** Storage key of the DTR/RTS sequence. */
const SEQUENCE_KEY = "serialbot.signalSequence";
/** Storage key of the highlighted keywords. */
const KEYWORDS_KEY = "serialbot.keywords";

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
  /** Terminal emulator for terminal mode (replaceable in tests). */
  createScreen?: ScreenFactory;
  /** Chart for the Plot view (replaceable in tests). */
  createChart?: ChartFactory;
  /**
   * Set in a session's own window: shows only this session, without the
   * sidebar, connection bar and tabs.
   */
  pinned?: number | null;
  /** Called in a session's own window once its session is gone (closes the window). */
  onPinnedClosed?: () => void;
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
  /** Ids of bookmarked lines (lost when the view is re-rendered). */
  bookmarks: Set<number>;
  /** Plotted numbers, made when the Plot view is first opened for the session. */
  plot?: PlotData;
}

const nowUs = () => Date.now() * 1000;

const defaultCopy = (text: string) => navigator.clipboard.writeText(text);

export function App({
  backend,
  lang,
  copy = defaultCopy,
  storage = browserStorage(),
  createScreen = loadXtermScreen,
  createChart = loadUplotChart,
  pinned = null,
  onPinnedClosed,
}: Props) {
  // App preferences, read first: the language decides every string.
  const [prefs, setPrefs] = useState(() => loadPrefs(storage));
  const uiLang = resolveLanguage(prefs.language, lang);
  const t = strings(uiLang);
  useEffect(() => {
    document.documentElement.lang = uiLang === "zh" ? "zh-CN" : "en";
  }, [uiLang]);
  // The command window edits the same groups; each window follows the other.
  const [groups, setGroups] = useSharedGroups(storage, t.defaultGroup);
  const [showCommands, setShowCommands] = useState(false);
  const [showSequence, setShowSequence] = useState(false);
  const [testSeq, setTestSeq] = useState(() => loadSequence(storage));
  const [sequences, setSequences] = useState<Record<number, SequenceStatus>>(
    {},
  );
  const sequenceStarts = useRef<
    Record<number, { at: Date; script: string; port: string }>
  >({});
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
  const sessionsRef = useRef(sessions);
  sessionsRef.current = sessions;
  const [activeId, setActiveId] = useState<number | null>(pinned);
  // Main window: sessions shown in their own windows (their tabs are hidden).
  const [popped, setPopped] = useState<ReadonlySet<number>>(new Set());
  const poppedRef = useRef(popped);
  poppedRef.current = popped;
  // Whether this window shows a session: only its own in a session window,
  // all but the popped-out ones in the main window.
  const shows = useCallback(
    (id: number) => (pinned !== null ? id === pinned : !poppedRef.current.has(id)),
    [pinned],
  );
  // Whether the session list has been read once (a session window closes
  // when its session is not in it).
  const [listed, setListed] = useState(false);
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
  const [showLineNumbers, setShowLineNumbers] = useState(false);
  /** Sessions shown in terminal mode. */
  const [ttyIds, setTtyIds] = useState<ReadonlySet<number>>(new Set());
  /** Colors for terminal mode, read after each theme change. */
  const [screenTheme, setScreenTheme] = useState<ScreenTheme | null>(null);
  /** Sessions showing the Plot view. */
  const [plotIds, setPlotIds] = useState<ReadonlySet<number>>(new Set());
  const [chartTheme, setChartTheme] = useState<ChartTheme | null>(null);
  const [jumpTo, setJumpTo] = useState<{ id: number; seq: number } | null>(null);
  const [filterMatches, setFilterMatches] = useState(false);
  const [autoScroll, setAutoScroll] = useState(true);
  const [mcp, setMcp] = useState<McpStatus | null>(null);
  const [approvals, setApprovals] = useState<ApprovalRequest[]>([]);
  const [page, setPage] = useState<Page>("terminal");
  // The version is shown on the help page; it is only asked for there.
  const [appVersion, setAppVersion] = useState<string | null>(null);
  useEffect(() => {
    if (page !== "help" || appVersion !== null) return;
    backend.appVersion().then(setAppVersion, () => undefined);
  }, [backend, page, appVersion]);
  const openLink = (url: string) => {
    backend.openLink(url).catch((e: unknown) => setError(errorMessage(e)));
  };
  const [signals, setSignals] = useState<Record<number, SignalsInfo>>({});
  const storeSignals = useCallback(
    (info: SignalsInfo) => setSignals((all) => ({ ...all, [info.id]: info })),
    [],
  );
  const views = useRef(new Map<number, SessionView>());
  const [encoding, setEncoding] = useState("utf-8");
  const [aiCopied, setAiCopied] = useState(false);
  const [frameGapMs, setFrameGapMs] = useState(0);
  // Display settings also apply to sessions that appear later.
  const display = useRef({
    hex: false,
    encoding: "utf-8",
    frameGapMs: 0,
    ansi: true,
  });
  const [ansi, setAnsi] = useState(true);
  const [frozen, setFrozen] = useState<{
    id: number;
    lines: readonly Line[];
  } | null>(null);
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState("");
  /** Index into the matches; null follows the newest match. */
  const [matchPos, setMatchPos] = useState<number | null>(null);
  const [version, setVersion] = useState(0);
  const redraw = () => setVersion((v) => v + 1);

  // Sessions with news since they were last shown: their tab dot says so.
  const [activity, setActivity] = useState<ReadonlySet<number>>(new Set());
  const activeIdRef = useRef(activeId);
  activeIdRef.current = activeId;
  const noteActivity = (id: number) => {
    if (id === activeIdRef.current) return;
    setActivity((all) => (all.has(id) ? all : new Set(all).add(id)));
  };
  useEffect(() => {
    if (activeId === null) return;
    setActivity((all) => {
      if (!all.has(activeId)) return all;
      const next = new Set(all);
      next.delete(activeId);
      return next;
    });
  }, [activeId]);

  const viewOf = (id: number): SessionView => {
    let view = views.current.get(id);
    if (!view) {
      const terminal = new Terminal({
        ...DEFAULT_LIMITS,
        maxLines: prefs.maxLines,
      });
      terminal.setHex(display.current.hex);
      terminal.setEncoding(display.current.encoding);
      terminal.setFrameGap(display.current.frameGapMs);
      terminal.setAnsi(display.current.ansi);
      view = {
        terminal,
        cursor: 0,
        rx: 0,
        tx: 0,
        reading: false,
        again: false,
        ended: false,
        bookmarks: new Set(),
      };
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
          if (chunk.entries.length > 0) noteActivity(id);
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
            if (chunk.state === "closed") {
              // Closed on request: say so, and how to continue.
              view.terminal.info(t.portClosed, nowUs());
            } else {
              view.terminal.info(chunk.error ?? t.deviceGone, nowUs());
              noteActivity(id);
            }
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
    // viewOf, redraw and noteActivity only touch refs and state setters.
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
        active !== null && ids.has(active) && shows(active)
          ? active
          : (list.filter((s) => shows(s.id)).at(-1)?.id ?? null),
      );
      setListed(true);
    },
    [pull, shows],
  );

  // A session window closes once its session is gone.
  const pinnedGone = pinned !== null && listed && !sessions.some((s) => s.id === pinned);
  useEffect(() => {
    if (pinnedGone) onPinnedClosed?.();
  }, [pinnedGone, onPinnedClosed]);

  // Keeps the selected port while it is still present.
  const applyPorts = useCallback((list: PortInfo[]) => {
    setPorts(list);
    // An open port stays selected even if the system does not list it.
    setSettings((s) =>
      list.some((p) => p.path === s.port) ||
      sessionsRef.current.some((session) => session.port === s.port)
        ? s
        : { ...s, port: list[0]?.path ?? "" },
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
        keep(await backend.onMcpChanged(setMcp));
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

  // Theme and font are applied to the document.
  useEffect(() => savePrefs(storage, prefs), [storage, prefs]);
  useEffect(() => {
    const media = window.matchMedia?.("(prefers-color-scheme: dark)");
    const apply = () => {
      applyAppearance(document.documentElement, prefs, media?.matches ?? false);
      // Terminal mode draws with the palette now in effect.
      setScreenTheme(themeFromCss(getComputedStyle(document.documentElement)));
      setChartTheme(chartThemeFromCss(getComputedStyle(document.documentElement)));
    };
    apply();
    media?.addEventListener("change", apply);
    return () => media?.removeEventListener("change", apply);
  }, [prefs]);
  useEffect(() => {
    backend.setAlwaysOnTop(prefs.alwaysOnTop).catch((e: unknown) => setError(errorMessage(e)));
  }, [backend, prefs.alwaysOnTop]);
  useEffect(() => {
    for (const view of views.current.values()) view.terminal.setMaxLines(prefs.maxLines);
    redraw();
  }, [prefs.maxLines]);
  // The DTR/RTS sequence last typed, kept across restarts.
  const [sequence, setSequence] = useState(() => readItem(storage, SEQUENCE_KEY) ?? "");
  useEffect(() => writeItem(storage, SEQUENCE_KEY, sequence), [storage, sequence]);
  // Keywords marked in the log, one per line, kept across restarts.
  const [keywords, setKeywords] = useState(() => readItem(storage, KEYWORDS_KEY) ?? "");
  useEffect(() => writeItem(storage, KEYWORDS_KEY, keywords), [storage, keywords]);
  const highlight = useMemo(() => makeHighlighter(parseHighlights(keywords)), [keywords]);

  // A run does not outlive the window.

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
  // The open session on the port in the connection bar, which Close closes.
  const portSession =
    ("path" in target && sessions.find((s) => s.port === target.path && s.state === "open")) ||
    null;

  // The connection bar follows the selected tab, so Close closes the port
  // being looked at. It can still be changed to open or close another one.
  useEffect(() => {
    const session = sessionsRef.current.find((s) => s.id === activeId);
    if (!session) return;
    const { kind, host, port, localPort } = endpointFields(session.port);
    setSettings((s) =>
      kind === "serial"
        ? { ...s, kind, port: session.port }
        : { ...s, kind, host, netPort: port, localPort },
    );
  }, [activeId]);

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
        // The tab being looked at continues if it is on this port and closed.
        ...(active && active.port === target.path && active.state !== "open"
          ? { session: active.id }
          : {}),
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

  // Closes the port but keeps the tab and its log; Open continues it.
  const release = async (id: number) => {
    setBusy(true);
    try {
      await backend.releaseSession(id);
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

  // Main window: shows a session in its own window and hides its tab until
  // that window closes; then the tab comes back, selected.
  const popOut = (id: number) => {
    const session = sessions.find((s) => s.id === id);
    if (!session || pinned !== null) return;
    const title = `${tabLabel(session.port, session.alias)} — SerialBot`;
    backend
      .openSessionWindow(id, title, () => {
        setPopped((all) => {
          const next = new Set(all);
          next.delete(id);
          return next;
        });
        setActiveId(id);
      })
      .then(
        () => {
          const next = new Set(poppedRef.current).add(id);
          poppedRef.current = next;
          setPopped(next);
          setActiveId((current) =>
            current === id || current === null
              ? (sessions.filter((s) => s.id !== id && !next.has(s.id)).at(-1)?.id ?? null)
              : current,
          );
        },
        (e: unknown) => setError(errorMessage(e)),
      );
  };

  // An agent's request is asked in one window only: the one showing its
  // session (the main window for new ports and sessions without a window).
  const asksHere = (request: ApprovalRequest) =>
    pinned !== null
      ? request.session === pinned
      : request.session === null || !popped.has(request.session);

  // Jobs may have been started in another window: read their state when a
  // session is shown.
  useEffect(() => {
    if (activeId === null) return;
    const id = activeId;
    const report = (e: unknown) => setError(errorMessage(e));
    backend.repeatStatus(id).then((r) => setRepeats((all) => ({ ...all, [id]: r })), report);
    backend.captureStatus(id).then((c) => setCaptures((all) => ({ ...all, [id]: c })), report);
    backend.fileSendStatus(id).then((f) => setFileSends((all) => ({ ...all, [id]: f })), report);
  }, [backend, activeId]);

  // Terminal mode: keys go to the device as typed; the device echoes.
  const typeIntoDevice = async (data: string) => {
    if (!active || !activeOpen) return;
    try {
      const bytes =
        encoding === "utf-8"
          ? new TextEncoder().encode(data)
          : Uint8Array.from(await backend.encodeText(data, encoding));
      await backend.writeData(active.id, bytes);
    } catch (e) {
      setError(errorMessage(e));
    }
  };
  // The Plot view: numbers from received lines, kept from the first opening on.
  const togglePlot = (on: boolean) => {
    if (activeId === null || !activeView) return;
    if (on && !activeView.plot) {
      const data = new PlotData();
      for (const chunk of activeView.terminal.receivedChunks()) {
        data.push(chunk.bytes, chunk.timestampUs);
      }
      activeView.terminal.subscribe((event) => {
        if (event.kind === "rx") data.push(event.bytes, event.timestampUs);
        else data.clear();
      });
      activeView.plot = data;
    }
    setPlotIds((ids) => {
      const next = new Set(ids);
      if (on) next.add(activeId);
      else next.delete(activeId);
      return next;
    });
  };
  const toggleTty = (on: boolean) => {
    if (activeId === null) return;
    setTtyIds((ids) => {
      const next = new Set(ids);
      if (on) next.add(activeId);
      else next.delete(activeId);
      return next;
    });
  };
  const screenOptions = useMemo(
    () => ({
      fontSize: prefs.fontSize,
      theme: screenTheme ?? themeFromCss(getComputedStyle(document.documentElement)),
    }),
    [prefs.fontSize, screenTheme],
  );

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
  const commandRunner = useCommandRunner({
    backend,
    encoding,
    invalidHex: t.invalidHex,
    onResult: setError,
    errorMessage,
  });
  const sendCommand = (c: Command) => (active ? commandRunner.send(active.id, c) : undefined);
  // Runs a group on the session selected when it starts.
  const runGroup = (group: CommandGroup) =>
    active ? commandRunner.runGroup(active.id, group) : undefined;

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

  const startFileSend = async ({ protocol, chunk, delayMs }: FileSendOptions) => {
    if (!active) return;
    const id = active.id;
    try {
      const path = await backend.chooseOpenFile();
      if (!path) return;
      const status = await backend.sendFile(id, path, chunk, delayMs, protocol);
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

  useEffect(() => {
    saveSequence(storage, testSeq);
  }, [storage, testSeq]);

  // While test sequences run, refresh their tally like timed sends.
  const sequenceIds = Object.entries(sequences)
    .filter(([, q]) => q?.running)
    .map(([id]) => Number(id))
    .join(",");
  useEffect(() => {
    if (sequenceIds === "") return undefined;
    const timer = setInterval(() => {
      for (const id of sequenceIds.split(",").map(Number)) {
        backend.sequenceStatus(id).then(
          (status) => setSequences((all) => ({ ...all, [id]: status })),
          (e: unknown) => setError(errorMessage(e)),
        );
      }
    }, REPEAT_POLL_MS);
    return () => clearInterval(timer);
  }, [backend, sequenceIds]);

  const startSequence = async () => {
    if (!active) return;
    const { id, port } = active;
    try {
      const status = await backend.startSequence(
        id,
        port,
        testSeq.script,
        send.ending,
        encoding,
        testSeq.runs > 0 ? testSeq.runs : null,
        !testSeq.keepGoing,
        testSeq.gapMs,
      );
      sequenceStarts.current[id] = {
        at: new Date(),
        script: testSeq.script,
        port,
      };
      setSequences((all) => ({ ...all, [id]: status }));
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const stopSequence = async (id: number) => {
    try {
      const status = await backend.stopSequence(id);
      setSequences((all) => ({ ...all, [id]: status }));
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const copySequenceReport = (id: number) => {
    const status = sequences[id];
    const start = sequenceStarts.current[id];
    if (!status || !start) return;
    copy(sequenceReport(t, status, start.port, start.script, start.at)).then(
      () => setError(null),
      (e: unknown) => setError(errorMessage(e)),
    );
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
    setFrozen(
      pause && activeView && activeId !== null
        ? { id: activeId, lines: activeView.terminal.view.slice() }
        : null,
    );
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
  // Bookmarks of lines still kept; trimmed or re-rendered lines drop out.
  const bookmarked = activeView ? live.filter((l) => activeView.bookmarks.has(l.id)) : [];
  const toggleBookmark = (id: number) => {
    const marks = activeView?.bookmarks;
    if (!marks) return;
    if (marks.has(id)) marks.delete(id);
    else marks.add(id);
    redraw();
  };
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
    <div className={`shell${pinned !== null ? " pinned" : ""}`}>
      {pinned === null && (
        <Sidebar t={t} page={page} onSelect={setPage} mcpRunning={mcpPort !== null} />
      )}
      <main className="main">
        {page === "terminal" ? (
          <>
            {pinned === null && (
              <ConnectionBar
                t={t}
                ports={ports}
                settings={settings}
                onChange={setSettings}
                onRefresh={() => void refreshPorts()}
                connected={portSession !== null}
                busy={busy}
                onOpen={() => void open()}
                onClose={() => portSession && void release(portSession.id)}
              />
            )}
            <SessionTabs
              t={t}
              sessions={sessions}
              activeId={activeId}
              onSelect={setActiveId}
              onClose={(id) => void close(id)}
              popped={popped}
              onPopOut={popOut}
              activity={activity}
              header={
                pinned !== null &&
                active && (
                  <div className="pinned-head">
                    <span className={`dot ${active.state}`} aria-hidden="true" />
                    <span className="pinned-name">
                      {active.alias ? `${active.port} (${active.alias})` : active.port}
                    </span>
                    <span className="spacer" />
                    <button
                      type="button"
                      className="chip"
                      title={t.backToMainHint}
                      onClick={() => void backend.closeWindow()}
                    >
                      {t.backToMain}
                    </button>
                    {active.state === "closed" ? (
                      <button
                        type="button"
                        className="chip"
                        disabled={busy}
                        onClick={() => void sessionCall((id) => backend.reconnectSession(id))}
                      >
                        {t.openPort}
                      </button>
                    ) : (
                      <button
                        type="button"
                        className="chip danger"
                        disabled={busy}
                        onClick={() => void release(active.id)}
                      >
                        {t.closePort}
                      </button>
                    )}
                  </div>
                )
              }
            >
              <Toggle
                label={t.plot}
                hint={t.plotHint}
                pressed={activeId !== null && plotIds.has(activeId)}
                onChange={togglePlot}
              />
              <Toggle
                label={t.tty}
                hint={t.ttyHint}
                pressed={activeId !== null && ttyIds.has(activeId)}
                onChange={toggleTty}
              />
              <Toggle
                label={t.showTimestamps}
                hint={t.showTimestampsHint}
                pressed={showTimestamps}
                onChange={setShowTimestamps}
              />
              <Toggle
                label={t.showLineNumbers}
                hint={t.showLineNumbersHint}
                pressed={showLineNumbers}
                onChange={setShowLineNumbers}
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
                className="chip"
                title={t.copyForAiHint}
                disabled={!active}
                onClick={() => {
                  if (!active) return;
                  const text = aiPrompt(visibleLines, active.alias ?? active.port, t.aiPrompt);
                  copy(text).then(
                    () => {
                      setAiCopied(true);
                      setTimeout(() => setAiCopied(false), 2000);
                    },
                    (e: unknown) => setError(errorMessage(e)),
                  );
                }}
              >
                {aiCopied ? t.copied : t.copyForAi}
              </button>
              <Bookmarks
                t={t}
                lines={bookmarked}
                onJump={(id) => {
                  // Stay at the bookmark instead of following new data.
                  setAutoScroll(false);
                  setJumpTo((j) => ({ id, seq: (j?.seq ?? 0) + 1 }));
                }}
                onRemove={toggleBookmark}
                onClear={() => {
                  activeView?.bookmarks.clear();
                  redraw();
                }}
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
                aria-label={t.sequence}
                title={t.testSequenceHint}
                aria-pressed={showSequence}
                onClick={() => setShowSequence((o) => !o)}
              >
                <RepeatIcon size={16} />
              </button>
              <button
                type="button"
                className="icon-button"
                aria-label={t.frames}
                title={t.framesHint}
                onClick={() =>
                  backend
                    .openFramesWindow(t.framesWindowTitle)
                    .catch((e: unknown) => setError(errorMessage(e)))
                }
              >
                <FramesIcon size={16} />
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
                keywords={keywords}
                onKeywords={setKeywords}
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
              {pinned === null && active && (
                <button
                  type="button"
                  className="icon-button"
                  aria-label={t.popOut}
                  title={t.popOutHint}
                  onClick={() => popOut(active.id)}
                >
                  ⧉
                </button>
              )}
            </SessionTabs>
            <div className="work">
              {active ? (
                <div className="log-area">
                  {activeId !== null && plotIds.has(activeId) && activeView?.plot && (
                    <PlotView
                      key={`plot-${activeId}`}
                      data={activeView.plot}
                      createChart={createChart}
                      theme={chartTheme ?? chartThemeFromCss(document.documentElement.style)}
                      emptyText={{ text: t.plotEmpty, justfloat: t.plotEmptyJustFloat }}
                      label={t.plotLabel(active.alias ?? active.port)}
                      formatLabel={t.plotFormat}
                      formatNames={{ text: t.plotFormatText, justfloat: t.plotFormatJustFloat }}
                      onFormat={(format) =>
                        activeView.plot?.setFormat(format, activeView.terminal.receivedChunks())
                      }
                      onError={setError}
                    />
                  )}
                  {activeId !== null && ttyIds.has(activeId) && activeView ? (
                    <TtyView
                      key={activeId}
                      terminal={activeView.terminal}
                      options={screenOptions}
                      onInput={(data) => void typeIntoDevice(data)}
                      createScreen={createScreen}
                      label={t.ttyLabel(active.alias ?? active.port)}
                      onError={setError}
                    />
                  ) : (
                    <>
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
                            setMatchPos((matchIndex + delta + matchIds.length) % matchIds.length)
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
                        showLineNumbers={showLineNumbers}
                        autoScroll={autoScroll && !paused}
                        version={version}
                        matches={new Set(matchIds)}
                        currentMatch={currentMatch}
                        highlight={highlight}
                        bookmarks={activeView?.bookmarks}
                        onToggleBookmark={toggleBookmark}
                        jumpTo={jumpTo}
                        bookmarkLabel={t.bookmarkLine}
                      />
                    </>
                  )}
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
                  run={commandRunner.run}
                  onRun={(g) => void runGroup(g)}
                  onStop={commandRunner.stop}
                  onPopOut={() =>
                    backend
                      .openCommandWindow(t.commandWindowTitle)
                      .catch((e: unknown) => setError(errorMessage(e)))
                  }
                  defaultEnding={send.ending}
                />
              )}
              {showSequence && (
                <SequencePanel
                  t={t}
                  settings={testSeq}
                  onChange={setTestSeq}
                  groups={groups}
                  enabled={activeOpen}
                  status={active ? (sequences[active.id] ?? null) : null}
                  onRun={() => void startSequence()}
                  onStop={() => active && void stopSequence(active.id)}
                  onCopyReport={() => active && copySequenceReport(active.id)}
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
        ) : page === "help" ? (
          <div className="page">
            <HelpPanel t={t} version={appVersion} onOpenLink={openLink} />
          </div>
        ) : page === "settings" ? (
          <div className="page">
            <SettingsPanel t={t} prefs={prefs} onChange={setPrefs} />
          </div>
        ) : (
          <div className="page">
            {mcp && (
              <McpPanel
                t={t}
                status={mcp}
                onApply={(s: McpSettings) => void mcpCall(() => backend.mcpApply(s))}
                onRegenerate={() => void mcpCall(() => backend.mcpRegenerateToken())}
                copy={copy}
                onClientConfig={(client) => backend.mcpClientConfig(client)}
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
              onClick={() => void sessionCall((id) => backend.setAiAccess(id, !active.aiAccess))}
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
              {fileSend.protocol && fileSend.sent === 0
                ? t.waitingForReceiver(fileSend.name ?? "", fileSend.protocol)
                : t.sendingFile(
                    fileSend.name ?? "",
                    fileSend.total > 0 ? Math.floor((fileSend.sent * 100) / fileSend.total) : 0,
                    formatBytes(fileSend.sent),
                    formatBytes(fileSend.total),
                  )}
              {fileSend.protocol && fileSend.sent > 0 && ` · ${fileSend.protocol}`}{" "}
              <button type="button" className="chip" onClick={() => void stopFileSend(active.id)}>
                {t.stopFile}
              </button>
            </span>
          )}
          {error && <span className="error">{error}</span>}
          <span className="spacer" />
          {pinned === null ? (
            <button type="button" className="link" onClick={() => setPage("ai")}>
              <span className={`dot ${mcpPort !== null ? "open" : "closed"}`} aria-hidden="true" />
              {t.mcpShort(mcpPort)}
            </button>
          ) : (
            <span>
              <span className={`dot ${mcpPort !== null ? "open" : "closed"}`} aria-hidden="true" />
              {t.mcpShort(mcpPort)}
            </span>
          )}
        </div>
      </main>
      <ApprovalDialog
        t={t}
        queue={approvals.filter(asksHere)}
        onAnswer={(id, decision) => {
          setApprovals((queue) => queue.filter((r) => r.id !== id));
          backend.answerApproval(id, decision).catch((e: unknown) => setError(errorMessage(e)));
        }}
      />
    </div>
  );
}
