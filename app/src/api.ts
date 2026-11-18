// Typed access to the Rust backend (app/src-tauri/src/commands.rs).
// Components receive a `Backend` so tests can substitute a fake.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

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
}

export interface LogEntry {
  seq: number;
  /** Microseconds since the Unix epoch. */
  timestampUs: number;
  kind: "rx" | "tx" | "marker";
  data: number[];
  label: string | null;
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

/** Progress of a session's timed send. */
export interface RepeatStatus {
  running: boolean;
  sent: number;
  error: string | null;
}

export interface McpSettings {
  enabled: boolean;
  /** Preferred port; the next free one of the following nine is used if taken. */
  port: number;
  token: string;
}

export interface McpStatus {
  settings: McpSettings;
  running: boolean;
  url: string | null;
  error: string | null;
}

export type Unlisten = () => void;

export interface Backend {
  listPorts(): Promise<PortInfo[]>;
  listSessions(): Promise<SessionInfo[]>;
  openSession(request: OpenRequest): Promise<SessionInfo>;
  closeSession(id: number): Promise<void>;
  writeData(id: number, data: Uint8Array): Promise<number>;
  readLog(id: number, cursor: number, maxBytes: number): Promise<LogChunk>;
  /** Called with the full list whenever sessions are opened, closed or change state. */
  onSessionsChanged(handler: (sessions: SessionInfo[]) => void): Promise<Unlisten>;
  /** Called with the ids of sessions that have new log entries or a new state. */
  onLogChanged(handler: (ids: number[]) => void): Promise<Unlisten>;
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
  /** Called when the line levels of an open session change. */
  onSignalsChanged(handler: (signals: SignalsInfo) => void): Promise<Unlisten>;
  mcpStatus(): Promise<McpStatus>;
  mcpApply(settings: McpSettings): Promise<McpStatus>;
  mcpRegenerateToken(): Promise<McpStatus>;
}

/** The backend inside the Tauri app. Errors reject with the message string. */
export const tauriBackend: Backend = {
  listPorts: () => invoke("list_ports"),
  listSessions: () => invoke("list_sessions"),
  openSession: (request) => invoke("open_session", { request }),
  closeSession: (id) => invoke("close_session", { id }),
  writeData: (id, data) => invoke("write_data", { id, data: Array.from(data) }),
  readLog: (id, cursor, maxBytes) => invoke("read_log", { id, cursor, maxBytes }),
  onSessionsChanged: (handler) =>
    listen<SessionInfo[]>("sessions-changed", (event) => handler(event.payload)),
  onLogChanged: (handler) => listen<number[]>("log-changed", (event) => handler(event.payload)),
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
  mcpStatus: () => invoke("mcp_status"),
  mcpApply: (settings) => invoke("mcp_apply", { settings }),
  mcpRegenerateToken: () => invoke("mcp_regenerate_token"),
};

/** The command that adds this server to Claude Code. */
export function claudeCommand(url: string, token: string): string {
  return `claude mcp add --transport http serialx ${url} --header "Authorization: Bearer ${token}"`;
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
