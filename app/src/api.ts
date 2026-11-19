// Typed access to the Rust backend (app/src-tauri/src/commands.rs).
// Components receive a `Backend` so tests can substitute a fake.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { SignalStep } from "./signals";

export type Parity = "none" | "odd" | "even";
export type FlowControl = "none" | "software" | "hardware";
export type SessionStateName = "open" | "disconnected" | "closed" | "failed";

export interface PortInfo {
  path: string;
  kind: "usb" | "bluetooth" | "pci" | "unknown";
  vid: number | null;
  pid: number | null;
  manufacturer: string | null;
  product: string | null;
  serialNumber: string | null;
}

export interface OpenRequest {
  port: string;
  baudRate: number;
  dataBits: number;
  parity: Parity;
  stopBits: number;
  flowControl: FlowControl;
}

export interface SessionInfo {
  id: number;
  /** Name given by an MCP client, if any. */
  alias: string | null;
  port: string;
  state: SessionStateName;
  error: string | null;
  /** Whether the port is reopened by itself after the device comes back. */
  autoReconnect: boolean;
  /** Whether AI agents (MCP) may use the session. */
  aiAccess: boolean;
}

export interface LogEntry {
  seq: number;
  /** Microseconds since the Unix epoch. */
  timestampUs: number;
  kind: "rx" | "tx" | "marker";
  data: number[];
  label: string | null;
  /** Whether an AI agent sent the data or set the marker. */
  ai?: boolean;
}

export interface LogChunk {
  entries: LogEntry[];
  next: number;
  missed: number;
  state: SessionStateName;
  error: string | null;
}

/** Modem line levels; `null` = unknown (output never set / input unreadable). */
export interface SignalsInfo {
  id: number;
  dtr: boolean | null;
  rts: boolean | null;
  /** False while RTS/CTS hardware flow control drives RTS. */
  rtsManual: boolean;
  cts: boolean | null;
  dsr: boolean | null;
  ri: boolean | null;
  cd: boolean | null;
}

/** "raw": received bytes as they arrived; "log": timestamped readable lines. */
export type ExportFormat = "raw" | "log";

/** State of a session's live capture to a file. */
export interface CaptureStatus {
  active: boolean;
  path: string | null;
  bytes: number;
  error: string | null;
}

/** Progress of a session's timed send. */
export interface RepeatStatus {
  running: boolean;
  sent: number;
  error: string | null;
}

/** Progress of a session's file send. */
export interface FileSendStatus {
  running: boolean;
  /** File name of the current or last send. */
  name: string | null;
  sent: number;
  total: number;
  error: string | null;
}

export interface McpSettings {
  enabled: boolean;
  /** Preferred port; the next free one of the following nine is used if taken. */
  port: number;
  token: string;
  /** Whether the user confirms each change an agent makes. */
  confirm: boolean;
}

/** An agent action waiting for the user's answer. */
export interface ApprovalRequest {
  id: number;
  /** Tool name, e.g. "io.write" or "command.call io.break". */
  tool: string;
  /** Target session; null when the agent opens a new port. */
  session: number | null;
  port: string;
  /** What is sent or changed, in one line; may be empty. */
  detail: string;
}

/** "session" also allows every later change on the same session. */
export type Decision = "once" | "session" | "deny";

export interface McpStatus {
  settings: McpSettings;
  running: boolean;
  url: string | null;
  error: string | null;
  /** Program stdio-only MCP clients start with the argument `mcp`. */
  stdioCommand: string | null;
}

export type Unlisten = () => void;

export interface Backend {
  listPorts(): Promise<PortInfo[]>;
  listSessions(): Promise<SessionInfo[]>;
  openSession(request: OpenRequest): Promise<SessionInfo>;
  closeSession(id: number): Promise<void>;
  /** Reopens the port of a disconnected or failed session; its log continues. */
  reconnectSession(id: number): Promise<void>;
  setAutoReconnect(id: number, on: boolean): Promise<void>;
  /** Lets AI agents use the session, or withholds it from them. */
  setAiAccess(id: number, allowed: boolean): Promise<void>;
  /** Called with the new port list when a device is plugged in or removed. */
  onPortsChanged(handler: (ports: PortInfo[]) => void): Promise<Unlisten>;
  writeData(id: number, data: Uint8Array): Promise<number>;
  readLog(id: number, cursor: number, maxBytes: number): Promise<LogChunk>;
  /** Called with the full list whenever sessions are opened, closed or change state. */
  onSessionsChanged(handler: (sessions: SessionInfo[]) => void): Promise<Unlisten>;
  /** Called with the ids of sessions that have new log entries or a new state. */
  onLogChanged(handler: (ids: number[]) => void): Promise<Unlisten>;
  /** Asks where to save; null when the user cancels. */
  chooseSaveFile(defaultName: string, format: ExportFormat): Promise<string | null>;
  /** Asks for a file to open; null when the user cancels. */
  chooseOpenFile(): Promise<string | null>;
  /** Writes the file at `path` in chunks of `chunk` bytes, pausing `delayMs` after each. */
  sendFile(id: number, path: string, chunk: number, delayMs: number): Promise<FileSendStatus>;
  stopFileSend(id: number): Promise<FileSendStatus>;
  fileSendStatus(id: number): Promise<FileSendStatus>;
  /** Writes what the session's log holds now to `path`. */
  exportLog(id: number, path: string, format: ExportFormat): Promise<{ bytes: number }>;
  /** Saves the log to `path` and keeps appending new entries. */
  startCapture(id: number, path: string, format: ExportFormat): Promise<CaptureStatus>;
  stopCapture(id: number): Promise<CaptureStatus>;
  captureStatus(id: number): Promise<CaptureStatus>;
  /** Sends `data` every `intervalMs`, `count` times or (null) until stopped. */
  startRepeat(
    id: number,
    data: Uint8Array,
    intervalMs: number,
    count: number | null,
  ): Promise<RepeatStatus>;
  stopRepeat(id: number): Promise<RepeatStatus>;
  repeatStatus(id: number): Promise<RepeatStatus>;
  /** Encodes text in a non-UTF-8 encoding (WHATWG label) for sending. */
  encodeText(text: string, encoding: string): Promise<number[]>;
  signals(id: number): Promise<SignalsInfo>;
  /** Drives DTR and/or RTS; an omitted line is left unchanged. */
  setSignals(id: number, levels: { dtr?: boolean; rts?: boolean }): Promise<SignalsInfo>;
  /** Runs a DTR/RTS sequence with pauses timed by the backend. */
  runSignals(id: number, steps: SignalStep[]): Promise<SignalsInfo>;
  /** Holds a BREAK for `ms` milliseconds (1 to 5000). */
  sendBreak(id: number, ms: number): Promise<void>;
  /** Called when the line levels of an open session change. */
  onSignalsChanged(handler: (signals: SignalsInfo) => void): Promise<Unlisten>;
  mcpStatus(): Promise<McpStatus>;
  mcpApply(settings: McpSettings): Promise<McpStatus>;
  mcpRegenerateToken(): Promise<McpStatus>;
  pendingApprovals(): Promise<ApprovalRequest[]>;
  answerApproval(id: number, decision: Decision): Promise<void>;
  /** Called with the whole queue whenever a request arrives or leaves. */
  onApprovalsChanged(handler: (queue: ApprovalRequest[]) => void): Promise<Unlisten>;
}

/** The backend inside the Tauri app. Errors reject with the message string. */
export const tauriBackend: Backend = {
  listPorts: () => invoke("list_ports"),
  listSessions: () => invoke("list_sessions"),
  openSession: (request) => invoke("open_session", { request }),
  closeSession: (id) => invoke("close_session", { id }),
  reconnectSession: (id) => invoke("reconnect", { id }),
  setAutoReconnect: (id, on) => invoke("set_auto_reconnect", { id, on }),
  setAiAccess: (id, allowed) => invoke("set_ai_access", { id, allowed }),
  onPortsChanged: (handler) =>
    listen<PortInfo[]>("ports-changed", (event) => handler(event.payload)),
  writeData: (id, data) => invoke("write_data", { id, data: Array.from(data) }),
  readLog: (id, cursor, maxBytes) => invoke("read_log", { id, cursor, maxBytes }),
  onSessionsChanged: (handler) =>
    listen<SessionInfo[]>("sessions-changed", (event) => handler(event.payload)),
  onLogChanged: (handler) => listen<number[]>("log-changed", (event) => handler(event.payload)),
  chooseSaveFile: (defaultName, format) =>
    save({
      defaultPath: defaultName,
      filters:
        format === "log"
          ? [{ name: "Log", extensions: ["log", "txt"] }]
          : [{ name: "Raw data", extensions: ["bin", "txt"] }],
    }),
  chooseOpenFile: async () => {
    const path = await open({ multiple: false, directory: false });
    return typeof path === "string" ? path : null;
  },
  sendFile: (id, path, chunk, delayMs) => invoke("send_file", { id, path, chunk, delayMs }),
  stopFileSend: (id) => invoke("stop_file_send", { id }),
  fileSendStatus: (id) => invoke("file_send_status", { id }),
  exportLog: (id, path, format) =>
    invoke("export_log", { id, path, format, utcOffsetMin: utcOffsetMin() }),
  startCapture: (id, path, format) =>
    invoke("start_capture", { id, path, format, utcOffsetMin: utcOffsetMin() }),
  stopCapture: (id) => invoke("stop_capture", { id }),
  captureStatus: (id) => invoke("capture_status", { id }),
  startRepeat: (id, data, intervalMs, count) =>
    invoke("start_repeat", { id, data: Array.from(data), intervalMs, count }),
  stopRepeat: (id) => invoke("stop_repeat", { id }),
  repeatStatus: (id) => invoke("repeat_status", { id }),
  encodeText: (text, encoding) => invoke("encode_text", { text, encoding }),
  signals: (id) => invoke("signals", { id }),
  setSignals: (id, levels) =>
    invoke("set_signals", { id, dtr: levels.dtr ?? null, rts: levels.rts ?? null }),
  onSignalsChanged: (handler) =>
    listen<SignalsInfo>("signals-changed", (event) => handler(event.payload)),
  runSignals: (id, steps) => invoke("run_signals", { id, steps }),
  sendBreak: (id, ms) => invoke("send_break", { id, ms }),
  mcpStatus: () => invoke("mcp_status"),
  mcpApply: (settings) => invoke("mcp_apply", { settings }),
  mcpRegenerateToken: () => invoke("mcp_regenerate_token"),
  pendingApprovals: () => invoke("pending_approvals"),
  answerApproval: (id, decision) => invoke("answer_approval", { id, decision }),
  onApprovalsChanged: (handler) =>
    listen<ApprovalRequest[]>("approvals-changed", (event) => handler(event.payload)),
};

/** The local time zone in minutes east of UTC (as the backend expects). */
export function utcOffsetMin(date = new Date()): number {
  return -date.getTimezoneOffset();
}

/** A file name for saving a session's log, e.g. "ttyUSB0_20261118-101338.log". */
export function saveFileName(port: string, format: ExportFormat, now = new Date()): string {
  const base = port.split(/[\\/]/).pop()?.replace(/[^\w.-]/g, "_") || "serial";
  const p = (n: number) => String(n).padStart(2, "0");
  const stamp =
    `${now.getFullYear()}${p(now.getMonth() + 1)}${p(now.getDate())}-` +
    `${p(now.getHours())}${p(now.getMinutes())}${p(now.getSeconds())}`;
  return `${base}_${stamp}.${format === "log" ? "log" : "bin"}`;
}

/** Human-readable size, e.g. "12.3 KB". */
export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** The command that adds this server to Claude Code. */
export function claudeCommand(url: string, token: string): string {
  return `claude mcp add --transport http serialbot ${url} --header "Authorization: Bearer ${token}"`;
}

/**
 * Claude Desktop entry (claude_desktop_config.json) that starts
 * `serialbot mcp`, the stdio bridge to the running app.
 */
export function claudeDesktopConfig(command: string): string {
  return JSON.stringify({ mcpServers: { serialbot: { command, args: ["mcp"] } } }, null, 2);
}

/** Text for a rejected backend call (Tauri rejects with plain strings). */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

/** Port label for the picker, e.g. "COM3 — CP2102 (10c4:ea60)". */
export function portLabel(port: PortInfo): string {
  const name = port.product ?? port.manufacturer;
  const ids =
    port.vid !== null && port.pid !== null
      ? `${hex4(port.vid)}:${hex4(port.pid)}`
      : null;
  if (name && ids) return `${port.path} — ${name} (${ids})`;
  if (name) return `${port.path} — ${name}`;
  if (ids) return `${port.path} (${ids})`;
  return port.path;
}

function hex4(n: number): string {
  return n.toString(16).padStart(4, "0");
}
