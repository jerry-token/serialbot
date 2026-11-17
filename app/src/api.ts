// Typed access to the Rust backend (app/src-tauri/src/commands.rs).
// Components receive a `Backend` so tests can substitute a fake.

import { invoke } from "@tauri-apps/api/core";

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

export interface Backend {
  listPorts(): Promise<PortInfo[]>;
  openSession(request: OpenRequest): Promise<SessionInfo>;
  closeSession(id: number): Promise<void>;
  writeData(id: number, data: Uint8Array): Promise<number>;
  readLog(id: number, cursor: number, maxBytes: number): Promise<LogChunk>;
}

/** The backend inside the Tauri app. Errors reject with the message string. */
export const tauriBackend: Backend = {
  listPorts: () => invoke("list_ports"),
  openSession: (request) => invoke("open_session", { request }),
  closeSession: (id) => invoke("close_session", { id }),
  writeData: (id, data) => invoke("write_data", { id, data: Array.from(data) }),
  readLog: (id, cursor, maxBytes) => invoke("read_log", { id, cursor, maxBytes }),
};

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
