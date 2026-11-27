// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
// The app with a fake backend in a plain browser, for e2e/layout.spec.ts:
// one USB port that opens, an MCP server that is off, everything else idle.
import { createRoot } from "react-dom/client";
import { App } from "../src/App";
import type {
  Backend,
  McpStatus,
  PortInfo,
  SessionInfo,
  SignalsInfo,
} from "../src/api";
import { detectLang } from "../src/i18n";
import "../src/styles.css";

let open = false;

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
    <App backend={backend} lang={detectLang(navigator.languages)} />,
  );
}
