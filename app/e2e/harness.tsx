// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
// The app with a fake backend in a plain browser, for e2e/layout.spec.ts:
// one USB port that opens, an MCP server that is off, everything else idle.
import { createRoot } from "react-dom/client";
import { App } from "../src/App";
import { FramesWindow } from "../src/FramesWindow";
import type {
  Backend,
  SequenceStatus,
  DecodedFrame,
  McpStatus,
  PortInfo,
  SessionInfo,
  SignalsInfo,
} from "../src/api";
import { detectLang } from "../src/i18n";
import "../src/styles.css";

// The frames window starts with a session to decode.
let open = window.location.hash === "#frames";

const session = (): SessionInfo => ({
  id: 1,
  alias: null,
  port: "COM3",
  state: open ? "open" : "closed",
  error: null,
  autoReconnect: true,
  aiAccess: true,
});

const port: PortInfo = {
  path: "COM3",
  kind: "usb",
  vid: 0x1a86,
  pid: 0x7523,
  manufacturer: "wch.cn",
  product: "USB-SERIAL CH340",
  serialNumber: null,
};

const sequence: SequenceStatus = {
  running: false,
  name: "COM3",
  runs: 1000,
  done: 1000,
  passed: 990,
  failed: 10,
  step: null,
  failures: Array.from({ length: 10 }, (_, i) => ({
    run: i * 97 + 3,
    line: 12,
    source: "expect ^\\+READY firmware-version:(\\S+) build-[0-9a-f]{12}$",
    reason: { kind: "timeout" as const },
  })),
  fastestMs: 1203,
  slowestMs: 98765,
  passedMs: 990 * 1500,
  elapsedMs: 1_600_000,
  stopped: false,
  error: null,
};

const STATUSES = ["ok", "badChecksum", "incomplete", "unknown"] as const;
const frames: DecodedFrame[] = Array.from({ length: 40 }, (_, i) => ({
  seq: i,
  timestampUs: 1_764_000_000_000_000 + i * 1000,
  direction: i % 2 === 0 ? "tx" : "rx",
  bytes: Array.from({ length: 40 }, (_, k) => ((i + k) % 256).toString(16).padStart(2, "0")).join(" "),
  name: "Read/Write Multiple Registers (17)",
  summary: "slave 247 · 0=65535 1=65535 2=65535 3=65535 4=65535 5=65535 6=65535 7=65535 …",
  fields: [
    { name: "slave", offset: 0, len: 1, value: 247, text: "247" },
    { name: "register 65535", offset: 3, len: 2, value: 65535, text: "65535 (0xFFFF)" },
    { name: "CRC", offset: 38, len: 2, value: 1, text: "0x0001, should be 0xFFFF" },
  ],
  status: STATUSES[i % 4] ?? "ok",
}));

const mcp: McpStatus = {
  settings: {
    enabled: false,
    port: 9123,
    token: "0123456789abcdef0123456789abcdef",
    confirm: true,
    listen: "local",
  },
  running: false,
  url: null,
  lanUrl: null,
  error: null,
  stdioCommand: "C:\\Program Files\\SerialBot\\serialbot.exe",
};

const signals: SignalsInfo = {
  id: 1,
  dtr: true,
  rts: true,
  rtsManual: true,
  cts: null,
  dsr: null,
  ri: null,
  cd: null,
};

const fake: Record<string, unknown> = {
  listPorts: async () => [port],
  listSessions: async () => (open ? [session()] : []),
  openSession: async () => {
    open = true;
    return session();
  },
  readLog: async () => ({
    entries: [],
    next: 0,
    missed: 0,
    state: "open",
    error: null,
  }),
  mcpStatus: async () => mcp,
  appVersion: async () => "0.0.0",
  encodeText: async (text: string) => [...new TextEncoder().encode(text)],
  signals: async () => signals,
  setSignals: async () => signals,
  runSignals: async () => signals,
  fileSendStatus: async () => ({
    running: false,
    name: null,
    sent: 0,
    total: 0,
    error: null,
  }),
  captureStatus: async () => ({ running: false }),
  repeatStatus: async () => ({ running: false, sent: 0 }),
  pendingApprovals: async () => [],
  // The editor: a long description and a preview of every frame.
  protocolSource: async () => ({
    name: "a-rather-long-protocol-file-name-for-the-board",
    text: `name = "Board"\n${"# a long comment line that goes on and on to test wrapping\n".repeat(30)}`,
  }),
  previewProtocol: async () => frames.slice(0, 6),
  saveProtocol: async (name: string) => `user:${name}`,
  // Frames with long names and summaries, every status, for the frames window.
  protocols: async () => [
    { id: "modbus-rtu", name: "Modbus RTU" },
    { id: "slcan", name: "SLCAN" },
  ],
  readFrames: async (_id: number, _p: string, _l: string, after: number) => ({
    frames: after > 0 ? [] : frames,
    next: frames.length,
    missed: 0,
  }),
  // A finished sequence with long failures, so its panel is seen full.
  startSequence: async () => sequence,
  sequenceStatus: async () => sequence,
  stopSequence: async () => sequence,
  // A long path and snippet, so the layout test sees them in a small window.
  mcpClientConfig: async () => ({
    client: "Claude Desktop",
    path: "C:\\Users\\someone-with-a-long-name\\AppData\\Roaming\\Claude\\claude_desktop_config.json",
    section: "mcpServers",
    snippet: JSON.stringify(
      { mcpServers: { serialbot: { command: "C:\\Program Files\\SerialBot\\serialbot.exe", args: ["mcp"] } } },
      null,
      2,
    ),
    error: null,
  }),
};

// Anything not listed: event subscriptions do nothing, other calls resolve to null.
const backend = new Proxy(fake, {
  get: (target, key: string) =>
    target[key] ??
    (key.startsWith("on") ? async () => () => {} : async () => null),
}) as unknown as Backend;

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    window.location.hash === "#frames" ? (
      <FramesWindow backend={backend} lang={detectLang(navigator.languages)} />
    ) : (
      <App backend={backend} lang={detectLang(navigator.languages)} />
    ),
  );
}
