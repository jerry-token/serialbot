import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { App } from "./App";
import type {
  ApprovalRequest,
  Decision,
  Backend,
  LogChunk,
  LogEntry,
  McpSettings,
  McpStatus,
  OpenRequest,
  PortInfo,
  SessionInfo,
  CaptureStatus,
  FileSendStatus,
  ExportFormat,
  RepeatStatus,
  SessionStateName,
  SignalsInfo,
} from "./api";

const PORTS: PortInfo[] = [
  {
    path: "COM3",
    kind: "usb",
    vid: 0x1a86,
    pid: 0x7523,
    manufacturer: null,
    product: "CH340",
    serialNumber: null,
  },
  {
    path: "COM4",
    kind: "unknown",
    vid: null,
    pid: null,
    manufacturer: null,
    product: null,
    serialNumber: null,
  },
];

interface FakeSession {
  info: SessionInfo;
  log: LogEntry[];
}

/**
 * In-memory backend modelled on the Rust side: one log per session, and
 * change events like the backend's watcher (sent asynchronously).
 */
class FakeBackend implements Backend {
  ports = PORTS;
  opened: OpenRequest[] = [];
  written: Array<[number, number[]]> = [];
  closed: number[] = [];
  sessions = new Map<number, FakeSession>();
  missed = 0;
  failOpen: string | null = null;
  failList = false;
  failWrite = false;
  failRead = false;
  failClose = false;
  failMcp: string | null = null;
  mcp: McpStatus = {
    settings: { enabled: false, port: 9633, token: "token-0123456789abcdef", confirm: true },
    running: false,
    url: null,
    error: null,
    stdioCommand: "/Applications/SerialBot.app/Contents/MacOS/serialbot",
  };
  lines: Record<number, SignalsInfo> = {};
  failSignals = false;
  signalListeners = new Set<(s: SignalsInfo) => void>();
  sessionListeners = new Set<(s: SessionInfo[]) => void>();
  logListeners = new Set<(ids: number[]) => void>();
  approvals: ApprovalRequest[] = [];
  answers: Array<[number, Decision]> = [];
  failAnswer = false;
  approvalListeners = new Set<(queue: ApprovalRequest[]) => void>();
  private nextId = 1;

  async listPorts() {
    if (this.failList) throw "permission denied: udev";
    return this.ports;
  }

  async listSessions() {
    return this.list();
  }

  async openSession(request: OpenRequest): Promise<SessionInfo> {
    if (this.failOpen) throw this.failOpen;
    this.opened.push(request);
    return this.addSession(request.port, null);
  }

  async closeSession(id: number) {
    this.closed.push(id);
    this.sessions.delete(id);
    this.notify([]);
    if (this.failClose) throw "no session s1";
  }

  async writeData(id: number, data: Uint8Array) {
    if (this.failWrite) throw "session is Disconnected";
    this.written.push([id, Array.from(data)]);
    this.append(id, "tx", Array.from(data));
    return 0;
  }

  async readLog(id: number, cursor: number, maxBytes: number): Promise<LogChunk> {
    if (this.failRead) throw "read failed";
    const session = this.sessions.get(id);
    if (!session) throw `no session s${id}`;
    const entries: LogEntry[] = [];
    let bytes = 0;
    for (const e of session.log.slice(cursor)) {
      if (entries.length > 0 && bytes + e.data.length > maxBytes) break;
      entries.push(e);
      bytes += e.data.length;
    }
    const missed = this.missed;
    this.missed = 0;
    return {
      entries,
      next: cursor + entries.length,
      missed,
      state: session.info.state,
      error: session.info.error,
    };
  }

  async onSessionsChanged(handler: (s: SessionInfo[]) => void) {
    this.sessionListeners.add(handler);
    return () => this.sessionListeners.delete(handler);
  }

  reconnects: number[] = [];
  failReconnect: string | null = null;
  portListeners = new Set<(p: PortInfo[]) => void>();

  async reconnectSession(id: number) {
    if (this.failReconnect) throw this.failReconnect;
    this.reconnects.push(id);
    this.setState(id, "open");
    this.appendMarker(id, "reconnected");
  }

  async setAiAccess(id: number, allowed: boolean) {
    const session = this.sessions.get(id);
    if (!session) throw `no session s${id}`;
    session.info = { ...session.info, aiAccess: allowed };
    this.notify([]);
  }

  /** What an agent does through MCP: sends data and sets a marker. */
  agentWrites(id: number, text: string, marker: string) {
    const session = this.sessions.get(id);
    if (!session) return;
    const data = Array.from(new TextEncoder().encode(text));
    for (const entry of [
      { kind: "tx" as const, data, label: null },
      { kind: "marker" as const, data: [], label: marker },
    ]) {
      const timestampUs = Date.now() * 1000;
      session.log.push({ seq: session.log.length, timestampUs, ai: true, ...entry });
    }
    this.notify([id]);
  }

  async setAutoReconnect(id: number, on: boolean) {
    const session = this.sessions.get(id);
    if (!session) throw `no session s${id}`;
    session.info = { ...session.info, autoReconnect: on };
    this.notify([]);
  }

  async onPortsChanged(handler: (p: PortInfo[]) => void) {
    this.portListeners.add(handler);
    return () => this.portListeners.delete(handler);
  }

  /** A device was plugged in or removed. */
  changePorts(ports: PortInfo[]) {
    this.ports = ports;
    setTimeout(() => {
      for (const l of this.portListeners) l(ports);
    }, 0);
  }

  appendMarker(id: number, text: string) {
    const session = this.sessions.get(id);
    if (!session) return;
    session.log.push({
      seq: session.log.length,
      timestampUs: Date.now() * 1000,
      kind: "marker",
      data: [],
      label: text,
    });
    this.notify([id]);
  }

  async onLogChanged(handler: (ids: number[]) => void) {
    this.logListeners.add(handler);
    return () => this.logListeners.delete(handler);
  }

  saveTo: string | null = "/home/me/out.log";
  exports: Array<[number, string, ExportFormat]> = [];
  capture: CaptureStatus = { active: false, path: null, bytes: 0, error: null };
  failExport: string | null = null;

  async chooseSaveFile(defaultName: string, format: ExportFormat) {
    this.lastSaveName = `${defaultName}|${format}`;
    return this.saveTo;
  }
  lastSaveName = "";

  async exportLog(id: number, path: string, format: ExportFormat) {
    if (this.failExport) throw this.failExport;
    this.exports.push([id, path, format]);
    return { bytes: 2048 };
  }

  async startCapture(_id: number, path: string, _format: ExportFormat) {
    this.capture = { active: true, path, bytes: 0, error: null };
    return this.capture;
  }

  async stopCapture(_id: number) {
    this.capture = { ...this.capture, active: false };
    return this.capture;
  }

  async captureStatus(_id: number) {
    return this.capture;
  }

  repeats = new Map<number, RepeatStatus & { data: number[]; intervalMs: number }>();

  async startRepeat(id: number, data: Uint8Array, intervalMs: number, count: number | null) {
    if (count !== null) throw "the UI never limits the count";
    await this.writeData(id, data);
    const job = { running: true, sent: 1, error: null, data: Array.from(data), intervalMs };
    this.repeats.set(id, job);
    return { running: true, sent: 1, error: null };
  }

  async stopRepeat(id: number) {
    const job = this.repeats.get(id);
    if (job) job.running = false;
    return { running: false, sent: job?.sent ?? 0, error: null };
  }

  async repeatStatus(id: number) {
    const job = this.repeats.get(id);
    return { running: job?.running ?? false, sent: job?.sent ?? 0, error: job?.error ?? null };
  }

  openPath: string | null = "/home/me/fw.bin";
  fileCalls: Array<[number, string, number, number]> = [];
  file: FileSendStatus = { running: false, name: null, sent: 0, total: 0, error: null };
  failFile: string | null = null;
  fileStops = 0;

  async chooseOpenFile() {
    return this.openPath;
  }

  async sendFile(id: number, path: string, chunk: number, delayMs: number) {
    if (this.failFile) throw this.failFile;
    this.fileCalls.push([id, path, chunk, delayMs]);
    this.file = { running: true, name: "fw.bin", sent: 0, total: 2048, error: null };
    return this.file;
  }

  async stopFileSend(_id: number) {
    this.fileStops += 1;
    this.file = { ...this.file, running: false };
    return this.file;
  }

  async fileSendStatus(_id: number) {
    return this.file;
  }

  async encodeText(text: string, encoding: string) {
    if (text.includes("😀")) throw `'😀' cannot be encoded in ${encoding}`;
    return Array.from(text).flatMap((c) => (c === "你" ? [0xc4, 0xe3] : [c.charCodeAt(0)]));
  }

  async signals(id: number) {
    if (this.failSignals) throw "not supported by this transport: modem lines";
    return this.linesOf(id);
  }

  async setSignals(id: number, levels: { dtr?: boolean; rts?: boolean }) {
    const session = this.sessions.get(id);
    if (session?.info.state !== "open") throw "session is Disconnected";
    const lines = { ...this.linesOf(id), ...levels };
    this.lines[id] = lines;
    return lines;
  }

  async onSignalsChanged(handler: (s: SignalsInfo) => void) {
    this.signalListeners.add(handler);
    return () => this.signalListeners.delete(handler);
  }

  /** The device changed its input lines. */
  changeLines(id: number, levels: Partial<SignalsInfo>) {
    this.lines[id] = { ...this.linesOf(id), ...levels };
    const info = this.lines[id];
    setTimeout(() => {
      for (const l of this.signalListeners) l(info);
    }, 0);
  }

  private linesOf(id: number): SignalsInfo {
    return (
      this.lines[id] ?? {
        id,
        dtr: null,
        rts: null,
        rtsManual: true,
        cts: false,
        dsr: false,
        ri: false,
        cd: null,
      }
    );
  }

  async mcpStatus() {
    return this.mcp;
  }

  async mcpApply(settings: McpSettings) {
    if (this.failMcp) throw this.failMcp;
    this.mcp = {
      settings,
      running: settings.enabled,
      url: settings.enabled ? `http://127.0.0.1:${settings.port}/mcp` : null,
      error: null,
      stdioCommand: this.mcp.stdioCommand,
    };
    return this.mcp;
  }

  async pendingApprovals() {
    return this.approvals;
  }

  async answerApproval(id: number, decision: Decision) {
    if (this.failAnswer) throw `request ${id} is no longer waiting`;
    this.answers.push([id, decision]);
    this.approvals = this.approvals.filter((r) => r.id !== id);
    for (const l of this.approvalListeners) l(this.approvals);
  }

  async onApprovalsChanged(handler: (queue: ApprovalRequest[]) => void) {
    this.approvalListeners.add(handler);
    return () => this.approvalListeners.delete(handler);
  }

  /** An agent asks to carry out an action. */
  agentAsks(request: Omit<ApprovalRequest, "id">) {
    this.approvals = [...this.approvals, { id: this.approvals.length + 1, ...request }];
    for (const l of this.approvalListeners) l(this.approvals);
  }

  async mcpRegenerateToken() {
    return this.mcpApply({ ...this.mcp.settings, token: "fresh-token-0123456789" });
  }

  /** A session opened by someone else, e.g. an MCP client. */
  addSession(port: string, alias: string | null): SessionInfo {
    const info: SessionInfo = {
      id: this.nextId++,
      alias,
      port,
      state: "open",
      error: null,
      autoReconnect: true,
      aiAccess: true,
    };
    this.sessions.set(info.id, { info, log: [] });
    this.notify([info.id]);
    return info;
  }

  receive(id: number, text: string) {
    this.append(id, "rx", Array.from(new TextEncoder().encode(text)));
  }

  receiveBytes(id: number, data: number[]) {
    this.append(id, "rx", data);
  }

  /** Received text with an explicit timestamp (microseconds). */
  receiveAt(id: number, text: string, timestampUs: number) {
    this.append(id, "rx", Array.from(new TextEncoder().encode(text)), timestampUs);
  }

  setState(id: number, state: SessionStateName, error: string | null = null) {
    const session = this.sessions.get(id);
    if (!session) return;
    session.info = { ...session.info, state, error };
    this.notify([id]);
  }

  private append(id: number, kind: "rx" | "tx", data: number[], timestampUs = Date.now() * 1000) {
    const session = this.sessions.get(id);
    if (!session) return;
    session.log.push({
      seq: session.log.length,
      timestampUs,
      kind,
      data,
      label: null,
    });
    this.notify([id]);
  }

  private list() {
    return [...this.sessions.values()].map((s) => s.info);
  }

  private notify(ids: number[]) {
    const list = this.list();
    setTimeout(() => {
      for (const l of this.sessionListeners) l(list);
      if (ids.length > 0) for (const l of this.logListeners) l(ids);
    }, 0);
  }
}

/** Local storage stand-in, so tests do not share saved commands. */
function memoryStorage(): Storage {
  const data = new Map<string, string>();
  return {
    get length() {
      return data.size;
    },
    clear: () => data.clear(),
    getItem: (k) => data.get(k) ?? null,
    key: (i) => [...data.keys()][i] ?? null,
    removeItem: (k) => void data.delete(k),
    setItem: (k, v) => void data.set(k, v),
  };
}

function setup(
  backend = new FakeBackend(),
  copy = vi.fn(async () => {}),
  storage: Storage = memoryStorage(),
) {
  const user = userEvent.setup();
  const view = render(<App backend={backend} lang="en" copy={copy} storage={storage} />);
  return { backend, user, copy, view, storage };
}

const panel = () => within(screen.getByRole("complementary", { name: "Commands" }));

/** Adds a command to the shown group through the editor. */
async function addCommand(
  user: ReturnType<typeof userEvent.setup>,
  name: string,
  text: string,
  options: { hex?: boolean; delayMs?: number } = {},
) {
  await user.click(panel().getByRole("button", { name: "+ Add" }));
  const name_ = panel().getByLabelText("Name");
  await user.clear(name_);
  await user.type(name_, name);
  await user.type(panel().getByLabelText("Content"), text);
  if (options.hex) await user.click(panel().getByRole("checkbox", { name: "Hex" }));
  if (options.delayMs !== undefined) {
    fireEvent.change(panel().getByLabelText("Delay ms"), {
      target: { value: String(options.delayMs) },
    });
  }
  await user.click(panel().getByRole("button", { name: "Done" }));
}

const FRAME = "Data bits, parity, stop bits, flow control";
const log = () => screen.getByRole("log");
const status = () => screen.getByRole("status");
const tabs = () => screen.queryAllByRole("tab").map((t) => t.textContent);

async function openPort(user: ReturnType<typeof userEvent.setup>, port = "COM3") {
  await waitFor(() => expect(screen.getByLabelText("Port")).toHaveValue("COM3"));
  await user.selectOptions(screen.getByLabelText("Port"), port);
  await user.click(screen.getByRole("button", { name: "Open" }));
  await waitFor(() => expect(status()).toHaveTextContent(`${port} · Open`));
}

describe("App", () => {
  it("lists ports and opens one with the chosen settings", async () => {
    const { backend, user } = setup();
    await waitFor(() => expect(screen.getByLabelText("Port")).toHaveValue("COM3"));
    expect(screen.getByRole("option", { name: "COM3 — CH340 (1a86:7523)" })).toBeInTheDocument();
    await user.selectOptions(screen.getByLabelText("Port"), "COM4");
    const baud = screen.getByLabelText("Baud");
    await user.clear(baud);
    await user.type(baud, "921600");
    const frame = screen.getByRole("button", { name: FRAME });
    expect(frame).toHaveTextContent("8N1");
    await user.click(frame);
    await user.selectOptions(screen.getByLabelText("Data"), "7");
    await user.selectOptions(screen.getByLabelText("Parity"), "even");
    await user.selectOptions(screen.getByLabelText("Stop"), "2");
    await user.selectOptions(screen.getByLabelText("Flow"), "hardware");
    expect(frame).toHaveTextContent("7E2 · RTS/CTS");
    await user.click(screen.getByRole("button", { name: "Open" }));
    await waitFor(() => expect(status()).toHaveTextContent("COM4 · Open"));
    expect(backend.opened).toEqual([
      {
        port: "COM4",
        baudRate: 921600,
        dataBits: 7,
        parity: "even",
        stopBits: 2,
        flowControl: "hardware",
      },
    ]);
    expect(tabs()).toEqual(["COM4"]);
    // Settings of an open port are locked; other ports can still be picked.
    expect(screen.getByLabelText("Baud")).toBeDisabled();
    expect(screen.getByLabelText("Port")).toBeEnabled();
    await user.selectOptions(screen.getByLabelText("Port"), "COM3");
    expect(screen.getByLabelText("Baud")).toBeEnabled();
    expect(screen.getByRole("button", { name: "Open" })).toBeInTheDocument();
  });

  it("shows received data pushed by events, with timestamps and views", async () => {
    const { backend, user } = setup();
    await openPort(user);
    act(() => backend.receive(1, "boot ok\r\nAB"));
    await waitFor(() => expect(log()).toHaveTextContent("boot ok"));
    expect(log().textContent).toMatch(/\[\d\d:\d\d:\d\d\.\d{3}\] boot ok/);
    expect(status()).toHaveTextContent("RX 11 · TX 0");

    await user.click(screen.getByLabelText("Show timestamps"));
    expect(log().textContent).not.toMatch(/\[\d\d:/);

    await user.click(screen.getByLabelText("Hex view"));
    expect(log()).toHaveTextContent("62 6f 6f 74 20 6f 6b 0d 0a 41 42");
    await user.click(screen.getByLabelText("Hex view"));
    expect(log()).toHaveTextContent("boot ok");

    await user.click(screen.getByLabelText("Auto-scroll to new data"));
    await user.click(screen.getByRole("button", { name: "Clear" }));
    expect(log().textContent).toBe("");
    expect(status()).toHaveTextContent("RX 0 · TX 0");
  });

  it("reads large bursts completely", async () => {
    const backend = new FakeBackend();
    const { user } = setup(backend);
    await openPort(user);
    const line = `${"x".repeat(1023)}\n`;
    act(() => {
      for (let i = 0; i < 300; i++) backend.receive(1, line);
    });
    await waitFor(() => expect(status()).toHaveTextContent("RX 307200"));
  });

  it("sends text with the line ending, and hex", async () => {
    const { backend, user } = setup();
    await openPort(user);
    const input = screen.getByLabelText("Send");
    await user.type(input, "AT{Enter}");
    await waitFor(() => expect(backend.written).toEqual([[1, [65, 84, 13, 10]]]));
    await waitFor(() => expect(log()).toHaveTextContent("→ AT"));

    await user.selectOptions(screen.getByLabelText("Line ending"), "lf");
    await user.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(backend.written[1]).toEqual([1, [65, 84, 10]]));

    await user.click(screen.getByLabelText("Show sent data"));
    await waitFor(() => expect(log()).not.toHaveTextContent("→ AT"));

    await user.click(screen.getByRole("checkbox", { name: "Hex" }));
    // HEX has no line ending; a checksum can be appended instead.
    expect(screen.queryByLabelText("Line ending")).not.toBeInTheDocument();
    await user.clear(input);
    await user.type(input, "01 ff{Enter}");
    await waitFor(() => expect(backend.written[2]).toEqual([1, [1, 255]]));

    await user.type(input, " z{Enter}");
    expect(status()).toHaveTextContent("Invalid hex");
    expect(backend.written).toHaveLength(3);

    const checksum = screen.getByLabelText("Checksum appended to HEX data");
    await user.selectOptions(checksum, "crc16-modbus");
    expect(checksum).toHaveAttribute("title", "Checksum appended to HEX data");
    await user.clear(input);
    await user.type(input, "01 03 00 00 00 01");
    expect(checksum).toHaveAttribute("title", "Appends 84 0a");
    await user.keyboard("{Enter}");
    await waitFor(() =>
      expect(backend.written[3]).toEqual([1, [1, 3, 0, 0, 0, 1, 0x84, 0x0a]]),
    );
    await user.selectOptions(checksum, "none");
    expect(checksum).toHaveAttribute("title", "Checksum appended to HEX data");
    await user.click(screen.getByRole("checkbox", { name: "Hex" }));
    expect(screen.getByLabelText("Line ending")).toHaveValue("lf");
  });

  it("shows sessions opened by MCP clients as tabs with their own logs", async () => {
    const { backend, user } = setup();
    await openPort(user, "COM3");
    act(() => backend.receive(1, "from ui\n"));
    await waitFor(() => expect(log()).toHaveTextContent("from ui"));

    act(() => {
      const s = backend.addSession("COM4", "esp32");
      backend.receive(s.id, "from agent\n");
    });
    await waitFor(() => expect(tabs()).toEqual(["COM3", "COM4 (esp32)"]));
    // The UI stays on its tab; the other session is read in the background.
    expect(log()).not.toHaveTextContent("from agent");
    await user.click(screen.getByRole("tab", { name: /COM4/ }));
    await waitFor(() => expect(log()).toHaveTextContent("from agent"));
    expect(log()).not.toHaveTextContent("from ui");
    expect(status()).toHaveTextContent("COM4 · Open");

    // Selecting the port of a session turns Open into Close.
    await user.selectOptions(screen.getByLabelText("Port"), "COM4");
    await user.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(tabs()).toEqual(["COM3"]));
    expect(backend.closed).toEqual([2]);
    expect(status()).toHaveTextContent("COM3 · Open");
  });

  it("picks up sessions that exist when the window opens", async () => {
    const backend = new FakeBackend();
    const s = backend.addSession("COM9", null);
    backend.receive(s.id, "already running\n");
    setup(backend);
    await waitFor(() => expect(tabs()).toEqual(["COM9"]));
    await waitFor(() => expect(log()).toHaveTextContent("already running"));
  });

  it("reports an unplugged device and closes the tab", async () => {
    const { backend, user } = setup();
    await openPort(user);
    act(() => {
      backend.receive(1, "last words\n");
      backend.setState(1, "disconnected");
    });
    await waitFor(() => expect(status()).toHaveTextContent("Disconnected"));
    await waitFor(() => expect(log()).toHaveTextContent("! Device disconnected"));
    expect(log()).toHaveTextContent("last words");
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Close session COM3" }));
    await waitFor(() => expect(tabs()).toEqual([]));
    expect(status()).toHaveTextContent("Closed");
    expect(backend.closed).toEqual([1]);
  });

  it("reconnects a lost session and reports a second disconnect", async () => {
    const { backend, user } = setup();
    await openPort(user);
    expect(screen.queryByRole("button", { name: "Reconnect" })).toBeNull();
    act(() => backend.setState(1, "disconnected"));
    await waitFor(() => expect(log()).toHaveTextContent("! Device disconnected"));
    await user.click(screen.getByRole("button", { name: "Reconnect" }));
    await waitFor(() => expect(status()).toHaveTextContent("COM3 · Open"));
    expect(backend.reconnects).toEqual([1]);
    await waitFor(() => expect(log()).toHaveTextContent("reconnected"));
    expect(screen.queryByRole("button", { name: "Reconnect" })).toBeNull();
    expect(tabs()).toEqual(["COM3"]);

    act(() => backend.setState(1, "disconnected"));
    await waitFor(() => expect(log().textContent?.match(/Device disconnected/g)).toHaveLength(2));
    backend.failReconnect = "port COM3 is already in use";
    await user.click(screen.getByRole("button", { name: "Reconnect" }));
    await waitFor(() => expect(status()).toHaveTextContent("port COM3 is already in use"));
  });

  it("turns auto-reconnect off and on for the selected session", async () => {
    const { backend, user } = setup();
    await openPort(user);
    const chip = () => screen.getByRole("button", { name: "Auto-reconnect" });
    expect(chip()).toHaveAttribute("aria-pressed", "true");
    await user.click(chip());
    await waitFor(() => expect(chip()).toHaveAttribute("aria-pressed", "false"));
    expect(backend.sessions.get(1)?.info.autoReconnect).toBe(false);
    await user.click(chip());
    await waitFor(() => expect(chip()).toHaveAttribute("aria-pressed", "true"));
    await user.click(screen.getByRole("button", { name: "Close session COM3" }));
    await waitFor(() => expect(tabs()).toEqual([]));
    expect(screen.queryByRole("button", { name: "Auto-reconnect" })).toBeNull();
  });

  it("refreshes the port list when devices are plugged in or removed", async () => {
    const { backend, user } = setup();
    await waitFor(() => expect(screen.getByLabelText("Port")).toHaveValue("COM3"));
    await user.selectOptions(screen.getByLabelText("Port"), "COM4");
    const com7 = { ...PORTS[1]!, path: "COM7" };
    act(() => backend.changePorts([PORTS[1]!, com7]));
    await waitFor(() => expect(screen.getByRole("option", { name: /^COM7/ })).toBeInTheDocument());
    expect(screen.getByLabelText("Port")).toHaveValue("COM4");
    expect(screen.queryByRole("option", { name: /^COM3/ })).toBeNull();
    act(() => backend.changePorts([com7]));
    await waitFor(() => expect(screen.getByLabelText("Port")).toHaveValue("COM7"));
  });

  it("keeps commands, sends one and runs a group in order", async () => {
    const { backend, user, storage, view } = setup();
    await openPort(user);
    await user.click(screen.getByRole("button", { name: "Commands" }));
    expect(panel().getByText("No commands yet")).toBeInTheDocument();
    expect(panel().getByRole("button", { name: "▶ Run all" })).toBeDisabled();
    await addCommand(user, "version", "AT+GMR", { delayMs: 0 });
    await addCommand(user, "raw", "01 02", { hex: true, delayMs: 0 });

    await user.click(panel().getByRole("button", { name: "Send version" }));
    const gmr = [...new TextEncoder().encode("AT+GMR\r\n")];
    await waitFor(() => expect(backend.written).toEqual([[1, gmr]]));
    await user.click(panel().getByRole("button", { name: "▶ Run all" }));
    await waitFor(() => expect(backend.written).toHaveLength(3));
    expect(backend.written.slice(1)).toEqual([
      [1, gmr],
      [1, [1, 2]],
    ]);
    await waitFor(() => expect(panel().getByRole("button", { name: "▶ Run all" })).toBeEnabled());

    // Saved for the next start.
    view.unmount();
    setup(new FakeBackend(), undefined, storage);
    await user.click(screen.getByRole("button", { name: "Commands" }));
    expect(panel().getByRole("button", { name: "Send version" })).toBeDisabled();
    expect(panel().getByRole("button", { name: "Send raw" })).toHaveTextContent("rawhex 01 02");
  });

  it("stops a run and reports a failed send", async () => {
    const { backend, user } = setup();
    await openPort(user);
    await user.click(screen.getByRole("button", { name: "Commands" }));
    await addCommand(user, "first", "a", { delayMs: 60_000 });
    await addCommand(user, "second", "b");
    await user.click(panel().getByRole("button", { name: "▶ Run all" }));
    const stop = await panel().findByRole("button", { name: "Stop 1/2" });
    expect(panel().getByRole("button", { name: "Send first" })).toBeDisabled();
    await user.click(stop);
    await waitFor(() => expect(panel().getByRole("button", { name: "▶ Run all" })).toBeEnabled());
    expect(backend.written).toHaveLength(1);

    backend.failWrite = true;
    await user.click(panel().getByRole("button", { name: "▶ Run all" }));
    await waitFor(() => expect(status()).toHaveTextContent("session is Disconnected"));
    expect(panel().getByRole("button", { name: "▶ Run all" })).toBeEnabled();
    await user.click(panel().getByRole("button", { name: "Send second" }));
    expect(status()).toHaveTextContent("session is Disconnected");
  });

  it("checks hex commands and uses the chosen text encoding", async () => {
    const { backend, user } = setup();
    await openPort(user);
    await user.click(screen.getByRole("button", { name: "Commands" }));
    await addCommand(user, "bad", "0g", { hex: true });
    await user.click(panel().getByRole("button", { name: "Send bad" }));
    await waitFor(() => expect(status()).toHaveTextContent("Invalid hex"));
    await addCommand(user, "hi", "你");
    await user.click(screen.getByRole("button", { name: "Display settings" }));
    await user.selectOptions(screen.getByLabelText("Encoding"), "gbk");
    await user.click(panel().getByRole("button", { name: "Send hi" }));
    await waitFor(() => expect(backend.written).toEqual([[1, [0xc4, 0xe3, 13, 10]]]));
  });

  it("edits, orders and deletes commands and groups", async () => {
    const { user } = setup();
    await user.click(screen.getByRole("button", { name: "Commands" }));
    await addCommand(user, "one", "1");
    await addCommand(user, "two", "2");
    const names = () =>
      panel()
        .queryAllByRole("button", { name: /^Send / })
        .map((b) => b.querySelector(".command-name")?.textContent);
    expect(names()).toEqual(["one", "two"]);

    await user.click(panel().getByRole("button", { name: "Edit two" }));
    expect(panel().getByRole("button", { name: "Down" })).toBeDisabled();
    await user.selectOptions(panel().getByLabelText("Line ending"), "lf");
    await user.click(panel().getByRole("button", { name: "Up" }));
    await user.click(panel().getByRole("button", { name: "Done" }));
    expect(names()).toEqual(["two", "one"]);
    await user.click(panel().getByRole("button", { name: "Edit two" }));
    expect(panel().getByLabelText("Line ending")).toHaveValue("lf");
    await user.click(panel().getByRole("button", { name: "Down" }));
    await user.click(panel().getByRole("button", { name: "Delete" }));
    await user.click(panel().getByRole("button", { name: "Click again to delete" }));
    expect(names()).toEqual(["one"]);

    // A new group starts in rename mode.
    await user.click(panel().getByRole("button", { name: "+" }));
    const groupName = panel().getByLabelText("Group name");
    expect(groupName).toHaveValue("Group 2");
    await user.clear(groupName);
    await user.type(groupName, "ESP32{Enter}");
    expect(panel().getByLabelText("Group")).toHaveDisplayValue("ESP32");
    expect(names()).toEqual([]);
    await user.selectOptions(panel().getByLabelText("Group"), "Default");
    expect(names()).toEqual(["one"]);

    // Deleting needs a second click; the last group is replaced by an empty one.
    await user.click(panel().getByRole("button", { name: "Edit" }));
    await user.click(panel().getByRole("button", { name: "Delete group" }));
    await user.click(panel().getByRole("button", { name: "Click again to delete" }));
    expect(panel().getByLabelText("Group")).toHaveDisplayValue("ESP32");
    await user.click(panel().getByRole("button", { name: "Edit" }));
    await user.click(panel().getByRole("button", { name: "Delete group" }));
    await user.click(panel().getByRole("button", { name: "Click again to delete" }));
    expect(panel().getByLabelText("Group")).toHaveDisplayValue("Default");
    expect(names()).toEqual([]);

    await user.click(screen.getByRole("button", { name: "Commands" }));
    expect(screen.queryByRole("complementary", { name: "Commands" })).toBeNull();
  });

  it("sends a file with the chosen chunk size and shows its progress", async () => {
    const { backend, user } = setup();
    const fileButton = () => screen.getByRole("button", { name: "Send a file" });
    expect(fileButton()).toBeDisabled();
    await openPort(user);
    await user.click(fileButton());
    const chunk = screen.getByLabelText("Chunk bytes");
    await user.clear(chunk);
    await user.type(chunk, "256");
    const pause = screen.getByLabelText("Pause ms");
    await user.clear(pause);
    await user.type(pause, "5");
    await user.click(screen.getByRole("button", { name: "Choose file and send…" }));
    await waitFor(() => expect(backend.fileCalls).toEqual([[1, "/home/me/fw.bin", 256, 5]]));
    expect(screen.queryByLabelText("Chunk bytes")).toBeNull();
    await waitFor(() => expect(status()).toHaveTextContent("Sending fw.bin 0% (0 B / 2.0 KB)"));
    expect(fileButton()).toBeDisabled();

    backend.file = { ...backend.file, sent: 1024 };
    await waitFor(() => expect(status()).toHaveTextContent("Sending fw.bin 50% (1.0 KB / 2.0 KB)"));
    await user.click(within(status()).getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(status()).not.toHaveTextContent("Sending"));
    expect(backend.fileStops).toBe(1);
    expect(fileButton()).toBeEnabled();
  });

  it("handles a cancelled dialog, bad options and failed file sends", async () => {
    const { backend, user } = setup();
    await openPort(user);
    backend.openPath = null;
    await user.click(screen.getByRole("button", { name: "Send a file" }));
    await user.click(screen.getByRole("button", { name: "Choose file and send…" }));
    await waitFor(() => expect(screen.queryByLabelText("Chunk bytes")).toBeNull());
    expect(backend.fileCalls).toEqual([]);

    backend.openPath = "/tmp/a.bin";
    await user.click(screen.getByRole("button", { name: "Send a file" }));
    fireEvent.change(screen.getByLabelText("Chunk bytes"), { target: { value: "999999" } });
    fireEvent.change(screen.getByLabelText("Pause ms"), { target: { value: "" } });
    await user.click(screen.getByRole("button", { name: "Choose file and send…" }));
    await waitFor(() => expect(backend.fileCalls).toEqual([[1, "/tmp/a.bin", 65536, 0]]));

    // The device went away during the send.
    backend.file = { ...backend.file, running: false, error: "session is Disconnected" };
    await waitFor(() => expect(status()).toHaveTextContent("session is Disconnected"));
    expect(status()).not.toHaveTextContent("Sending");

    backend.failFile = "the file is empty";
    await user.click(screen.getByRole("button", { name: "Send a file" }));
    expect(screen.getByLabelText("Chunk bytes")).toHaveValue(65536);
    await user.click(screen.getByRole("button", { name: "Choose file and send…" }));
    await waitFor(() => expect(status()).toHaveTextContent("the file is empty"));
  });

  it("resizes the send box from its top-right handle", async () => {
    setup();
    const box = screen.getByLabelText("Send");
    const handle = screen.getByRole("separator", { name: /resize the send box/ });
    const drag = (fromY: number, toY: number) => {
      fireEvent.pointerDown(handle, { clientY: fromY });
      act(() => {
        window.dispatchEvent(new MouseEvent("pointermove", { clientY: toY }));
        window.dispatchEvent(new MouseEvent("pointerup"));
      });
    };
    // jsdom has no layout, so the box starts at the minimum height.
    drag(500, 400);
    expect(box).toHaveStyle({ height: "148px" });
    // Released: later moves do nothing.
    act(() => window.dispatchEvent(new MouseEvent("pointermove", { clientY: 0 })));
    expect(box).toHaveStyle({ height: "148px" });
    fireEvent.keyDown(handle, { key: "ArrowUp" });
    expect(box).toHaveStyle({ height: "164px" });
    fireEvent.keyDown(handle, { key: "ArrowDown" });
    fireEvent.keyDown(handle, { key: "Enter" });
    expect(box).toHaveStyle({ height: "148px" });
    drag(1000, 0);
    expect(box).toHaveStyle({ height: "480px" });
    drag(0, 2000);
    expect(box).toHaveStyle({ height: "48px" });
  });

  it("opens TCP clients, TCP servers and UDP endpoints", async () => {
    const { backend, user } = setup();
    await waitFor(() => expect(screen.getByLabelText("Port")).toHaveValue("COM3"));
    const kind = screen.getByLabelText("Connection type");
    await user.selectOptions(kind, "tcp");
    expect(screen.queryByLabelText("Baud")).toBeNull();
    const open = () => screen.getByRole("button", { name: "Open" });

    await user.click(open());
    expect(status()).toHaveTextContent("Enter the host (an IP address or name)");
    await user.type(screen.getByLabelText("Host"), "192.168.1.10");
    await user.click(open());
    expect(status()).toHaveTextContent("Enter a port between 1 and 65535");
    await user.type(screen.getByLabelText("Port"), "8080");
    await user.click(open());
    await waitFor(() => expect(status()).toHaveTextContent("tcp://192.168.1.10:8080 · Open"));
    expect(backend.opened.at(-1)?.port).toBe("tcp://192.168.1.10:8080");
    // No modem lines on a network link; the type is locked while connected.
    expect(screen.queryByRole("button", { name: /^DTR/ })).toBeNull();
    expect(kind).toBeDisabled();
    expect(screen.getByRole("button", { name: "Close" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(kind).toBeEnabled());

    await user.selectOptions(kind, "tcp-server");
    expect(screen.queryByLabelText("Host")).toBeNull();
    await user.clear(screen.getByLabelText("Listen port"));
    await user.type(screen.getByLabelText("Listen port"), "9000");
    await user.click(open());
    await waitFor(() => expect(backend.opened.at(-1)?.port).toBe("tcp-server://:9000"));
    await user.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(kind).toBeEnabled());

    await user.selectOptions(kind, "udp");
    await user.clear(screen.getByLabelText("Host"));
    await user.clear(screen.getByLabelText("Port"));
    await user.click(open());
    expect(status()).toHaveTextContent("Enter a remote host and port, a local port");
    await user.type(screen.getByLabelText("Local port"), "7000");
    await user.click(open());
    await waitFor(() => expect(backend.opened.at(-1)?.port).toBe("udp://?local=:7000"));
    await user.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(kind).toBeEnabled());

    // Back to serial: the port list and line settings return.
    await user.selectOptions(kind, "serial");
    expect(screen.getByLabelText("Port")).toHaveValue("COM3");
    expect(screen.getByLabelText("Baud")).toBeInTheDocument();
  });

  it("marks what AI agents send and lets the user withhold a session", async () => {
    const { backend, user } = setup();
    await openPort(user);
    await user.type(screen.getByLabelText("Send"), "by hand{Enter}");
    act(() => backend.agentWrites(1, "AT+GMR\r\n", "agent check"));
    await waitFor(() => expect(log()).toHaveTextContent("AT+GMR"));
    const lines = [...log().querySelectorAll(".line")];
    const byText = (text: string) => lines.find((l) => l.textContent?.includes(text));
    expect(byText("by hand")).not.toHaveClass("ai");
    expect(byText("AT+GMR")).toHaveClass("ai");
    expect(byText("AT+GMR")?.querySelector(".ai-tag")).toHaveTextContent("AI");
    expect(byText("agent check")).toHaveClass("ai");

    const chip = () => screen.getByRole("button", { name: "AI access" });
    expect(chip()).toHaveAttribute("aria-pressed", "true");
    await user.click(chip());
    await waitFor(() => expect(chip()).toHaveAttribute("aria-pressed", "false"));
    expect(backend.sessions.get(1)?.info.aiAccess).toBe(false);
    await user.click(chip());
    await waitFor(() => expect(chip()).toHaveAttribute("aria-pressed", "true"));
  });

  it("shows the failure reason of a failed session", async () => {
    const { backend, user } = setup();
    await openPort(user);
    act(() => backend.setState(1, "failed", "I/O error: broken"));
    await waitFor(() => expect(status()).toHaveTextContent("Failed"));
    await waitFor(() => expect(log()).toHaveTextContent("! I/O error: broken"));
  });

  it("reports dropped log entries", async () => {
    const { backend, user } = setup();
    await openPort(user);
    act(() => {
      backend.missed = 7;
      backend.receive(1, "x");
    });
    await waitFor(() => expect(log()).toHaveTextContent("7 log entries were dropped"));
  });

  it("shows backend errors", async () => {
    const backend = new FakeBackend();
    backend.failList = true;
    const { user } = setup(backend);
    await waitFor(() => expect(status()).toHaveTextContent("permission denied: udev"));
    expect(screen.getByRole("option", { name: "No ports found" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open" })).toBeDisabled();

    backend.failList = false;
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(screen.getByLabelText("Port")).toHaveValue("COM3"));
    expect(status()).not.toHaveTextContent("permission denied");

    backend.failOpen = "port is busy, probably opened by another program: COM3";
    await user.click(screen.getByRole("button", { name: "Open" }));
    await waitFor(() => expect(status()).toHaveTextContent("port is busy"));
    expect(status()).toHaveTextContent("Closed");

    const baud = screen.getByLabelText("Baud");
    await user.clear(baud);
    await user.type(baud, "fast");
    await user.click(screen.getByRole("button", { name: "Open" }));
    expect(status()).toHaveTextContent("Baud rate must be a positive whole number");
  });

  it("reports failures to send, read and close", async () => {
    const { backend, user } = setup();
    await openPort(user);
    backend.failWrite = true;
    await user.type(screen.getByLabelText("Send"), "x{Enter}");
    await waitFor(() => expect(status()).toHaveTextContent("session is Disconnected"));

    backend.failClose = true;
    await user.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(status()).toHaveTextContent("no session s1"));
    expect(screen.getByRole("button", { name: "Open" })).toBeInTheDocument();

    backend.failRead = true;
    await user.click(screen.getByRole("button", { name: "Open" }));
    await waitFor(() => expect(status()).toHaveTextContent("read failed"));
  });

  it("keeps the selected port across refreshes while it exists", async () => {
    const { backend, user } = setup();
    await waitFor(() => expect(screen.getByLabelText("Port")).toHaveValue("COM3"));
    await user.selectOptions(screen.getByLabelText("Port"), "COM4");
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    expect(screen.getByLabelText("Port")).toHaveValue("COM4");
    backend.ports = [PORTS[0] as PortInfo];
    await user.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(screen.getByLabelText("Port")).toHaveValue("COM3"));
    const options = within(screen.getByLabelText("Port")).getAllByRole("option");
    expect(options).toHaveLength(1);
  });

  it("closes the frame popover on Escape and outside clicks", async () => {
    const { user } = setup();
    const frame = screen.getByRole("button", { name: FRAME });
    await user.click(frame);
    expect(screen.getByRole("dialog", { name: FRAME })).toBeInTheDocument();
    await user.click(screen.getByLabelText("Data"));
    expect(screen.getByRole("dialog", { name: FRAME })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    await user.click(frame);
    await user.click(log());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(log()).toHaveTextContent("No open port");
  });

  it("sends several lines, with Shift+Enter for new lines", async () => {
    const { backend, user } = setup();
    await openPort(user);
    const input = screen.getByLabelText("Send");
    await user.type(input, "a{Shift>}{Enter}{/Shift}b");
    expect(input).toHaveValue("a\nb");
    expect(backend.written).toEqual([]);
    // Enter while an input method is composing belongs to the IME.
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });
    expect(backend.written).toEqual([]);
    await user.type(input, "{Enter}");
    await waitFor(() => expect(backend.written).toEqual([[1, [97, 13, 10, 98, 13, 10]]]));
    await waitFor(() => expect(log()).toHaveTextContent(/→ a.*→ b/));
  });

  it("shows and drives the modem lines of the selected session", async () => {
    const { backend, user } = setup();
    await openPort(user);
    const dtr = screen.getByRole("button", { name: /^DTR/ });
    const rts = screen.getByRole("button", { name: /^RTS/ });
    await waitFor(() => expect(screen.getByRole("img", { name: /^CTS.*: 0$/ })).toBeInTheDocument());
    expect(screen.getByRole("img", { name: /^CD.*: unknown$/ })).toBeInTheDocument();
    expect(dtr).toHaveAttribute("aria-pressed", "false");
    expect(dtr).toHaveAttribute("title", expect.stringContaining("unknown"));

    await user.click(dtr);
    await waitFor(() => expect(dtr).toHaveAttribute("aria-pressed", "true"));
    expect(backend.lines[1]?.dtr).toBe(true);
    await user.click(dtr);
    await waitFor(() => expect(dtr).toHaveAttribute("aria-pressed", "false"));
    expect(backend.lines[1]?.dtr).toBe(false);
    await user.click(rts);
    await waitFor(() => expect(backend.lines[1]?.rts).toBe(true));

    act(() => backend.changeLines(1, { cts: true }));
    await waitFor(() => expect(screen.getByRole("img", { name: /^CTS.*: 1$/ })).toBeInTheDocument());

    act(() => backend.setState(1, "disconnected"));
    await waitFor(() => expect(dtr).toBeDisabled());
  });

  it("locks RTS while hardware flow control drives it", async () => {
    const backend = new FakeBackend();
    backend.lines[1] = {
      id: 1,
      dtr: null,
      rts: null,
      rtsManual: false,
      cts: false,
      dsr: false,
      ri: false,
      cd: false,
    };
    const { user } = setup(backend);
    await openPort(user);
    const rts = screen.getByRole("button", { name: /^RTS/ });
    await waitFor(() => expect(rts).toBeDisabled());
    expect(rts).toHaveAttribute("title", expect.stringContaining("hardware flow control"));
    expect(screen.getByRole("button", { name: /^DTR/ })).toBeEnabled();
  });

  it("reports modem line errors", async () => {
    const backend = new FakeBackend();
    backend.failSignals = true;
    const { user } = setup(backend);
    await openPort(user);
    await waitFor(() => expect(status()).toHaveTextContent("modem lines"));
    backend.failSignals = false;
    backend.sessions.get(1)!.info = { ...backend.sessions.get(1)!.info, state: "failed" };
    // Setting fails once the session is no longer open on the backend side.
    await user.click(screen.getByRole("button", { name: /^DTR/ }));
    await waitFor(() => expect(status()).toHaveTextContent("session is Disconnected"));
  });

  it("decodes and encodes with the chosen encoding", async () => {
    const { backend, user } = setup();
    await openPort(user);
    act(() => backend.receiveBytes(1, [0xce, 0xc2, 0xb6, 0xc8, 0x0a]));
    await waitFor(() => expect(status()).toHaveTextContent("RX 5"));
    expect(log()).not.toHaveTextContent("温度");
    await user.click(screen.getByRole("button", { name: "Display settings" }));
    await user.selectOptions(screen.getByLabelText("Encoding"), "gbk");
    expect(log()).toHaveTextContent("温度");

    await user.type(screen.getByLabelText("Send"), "你{Enter}");
    await waitFor(() => expect(backend.written).toEqual([[1, [0xc4, 0xe3, 13, 10]]]));
    await user.type(screen.getByLabelText("Send"), "😀{Enter}");
    await waitFor(() => expect(status()).toHaveTextContent("cannot be encoded in gbk"));
    // Hex input ignores the text encoding.
    await user.clear(screen.getByLabelText("Send"));
    await user.click(screen.getByRole("checkbox", { name: "Hex" }));
    await user.type(screen.getByLabelText("Send"), "ff{Enter}");
    await waitFor(() => expect(backend.written[1]).toEqual([1, [0xff]]));
  });

  it("shows ANSI colors and can show the raw codes instead", async () => {
    const { backend, user } = setup();
    await openPort(user);
    act(() => backend.receive(1, "\u001b[1;32muart:~$ \u001b[m\r\n\u001b[7minv\u001b[m\n"));
    await waitFor(() => expect(log()).toHaveTextContent("uart:~$"));
    const prompt = within(log()).getByText("uart:~$");
    expect(prompt.tagName).toBe("SPAN");
    // jsdom cannot resolve CSS variables, so check the inline style itself.
    expect(prompt.getAttribute("style")).toBe("color: var(--ansi-2); font-weight: 600;");
    expect(within(log()).getByText("inv").getAttribute("style")).toBe(
      "color: var(--log-bg); background-color: var(--rx);",
    );
    expect(log()).not.toHaveTextContent("[1;32m");

    await user.click(screen.getByRole("button", { name: "Display settings" }));
    await user.click(screen.getByLabelText("ANSI colors"));
    expect(log()).toHaveTextContent("\u241b[1;32muart:~$");
  });

  it("splits frames after a pause", async () => {
    const { backend, user } = setup();
    await openPort(user);
    act(() => {
      backend.receiveAt(1, "AB", 1_000);
      backend.receiveAt(1, "CD", 900_000);
    });
    await waitFor(() => expect(log()).toHaveTextContent("ABCD"));
    await user.click(screen.getByRole("button", { name: "Display settings" }));
    const gap = screen.getByLabelText("Frame gap (ms)");
    await user.clear(gap);
    await user.type(gap, "100");
    await waitFor(() => expect(log().querySelectorAll(".line.rx")).toHaveLength(2));
    await user.clear(gap);
    await user.type(gap, "0");
    await waitFor(() => expect(log().querySelectorAll(".line.rx")).toHaveLength(1));
    expect(log()).toHaveTextContent("ABCD");
  });

  it("pauses the view while data keeps arriving", async () => {
    const { backend, user } = setup();
    await openPort(user);
    act(() => backend.receive(1, "one\n"));
    await waitFor(() => expect(log()).toHaveTextContent("one"));
    await user.click(screen.getByLabelText("Freeze the view; data is still received"));
    act(() => backend.receive(1, "two\nthree\n"));
    await waitFor(() => expect(screen.getByText("Paused · 2 new lines")).toBeInTheDocument());
    expect(log()).not.toHaveTextContent("two");
    await user.click(screen.getByLabelText("Freeze the view; data is still received"));
    expect(log()).toHaveTextContent("three");
  });

  it("searches the log with Ctrl+F", async () => {
    const { backend, user } = setup();
    await openPort(user);
    act(() => backend.receive(1, "temp=21\nhum=40\nTEMP=22\ntemp=23\n"));
    await waitFor(() => expect(log()).toHaveTextContent("temp=23"));
    await user.keyboard("{Control>}f{/Control}");
    const box = screen.getByRole("searchbox", { name: "Search" });
    expect(box).toHaveFocus();
    await user.type(box, "temp");
    expect(screen.getByRole("search")).toHaveTextContent("3 / 3");
    const current = () => log().querySelector(".line.current")?.textContent ?? "";
    expect(current()).toContain("temp=23");
    expect(log().querySelectorAll(".line.match")).toHaveLength(3);
    await user.keyboard("{Enter}");
    expect(current()).toContain("temp=21");
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    expect(current()).toContain("temp=23");
    await user.click(screen.getByRole("button", { name: "Previous match" }));
    expect(current()).toContain("TEMP=22");
    await user.click(screen.getByRole("button", { name: "Next match" }));
    expect(screen.getByRole("search")).toHaveTextContent("3 / 3");
    await user.clear(box);
    await user.type(box, "nothing");
    expect(screen.getByRole("search")).toHaveTextContent("No matches");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("search")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Search" }));
    expect(screen.getByRole("search")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Close search" }));
    expect(screen.queryByRole("search")).not.toBeInTheDocument();
  });

  it("filters the log to matching lines", async () => {
    const { backend, user } = setup();
    await openPort(user);
    act(() => backend.receive(1, "temp=21\nhum=40\nTEMP=22\n"));
    await waitFor(() => expect(log()).toHaveTextContent("TEMP=22"));
    await user.keyboard("{Control>}f{/Control}");
    await user.click(screen.getByRole("checkbox", { name: "Only matching lines" }));
    // An empty query hides nothing.
    expect(log()).toHaveTextContent("hum=40");
    await user.type(screen.getByRole("searchbox", { name: "Search" }), "temp");
    expect(log()).not.toHaveTextContent("hum=40");
    expect(log().querySelectorAll(".line")).toHaveLength(2);
    expect(screen.getByRole("search")).toHaveTextContent("2 / 2");
    act(() => backend.receive(1, "hum=41\ntemp=23\n"));
    await waitFor(() => expect(log()).toHaveTextContent("temp=23"));
    expect(log()).not.toHaveTextContent("hum=41");
    await user.click(screen.getByRole("checkbox", { name: "Only matching lines" }));
    expect(log()).toHaveTextContent("hum=41");
    await user.click(screen.getByRole("checkbox", { name: "Only matching lines" }));
    // Closing the search shows everything again.
    await user.click(screen.getByRole("button", { name: "Close search" }));
    expect(log()).toHaveTextContent("hum=40");
  });

  it("shows the time between lines", async () => {
    const { backend, user } = setup();
    await openPort(user);
    act(() => {
      backend.receiveAt(1, "a\n", 1_000_000);
      backend.receiveAt(1, "b\n", 1_012_500);
      backend.receiveAt(1, "c\n", 3_612_300);
    });
    await waitFor(() => expect(log()).toHaveTextContent("c"));
    expect(log().querySelector(".delta")).toBeNull();
    await user.click(screen.getByRole("button", { name: /Show the time since/ }));
    const deltas = [...log().querySelectorAll(".delta")].map((d) => d.textContent?.trim());
    expect(deltas).toEqual(["", "+13ms", "+2.600s"]);
  });

  it("sends repeatedly until stopped", async () => {
    const { backend, user } = setup();
    await openPort(user);
    await user.type(screen.getByLabelText("Send"), "ping");
    const interval = screen.getByLabelText("Interval (ms)");
    expect(interval).toBeDisabled();
    await user.click(screen.getByRole("checkbox", { name: "Timed" }));
    await user.clear(interval);
    await user.type(interval, "250");
    await user.click(screen.getByRole("button", { name: "Start" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Stop (1)" })).toBeInTheDocument());
    expect(backend.repeats.get(1)).toMatchObject({ intervalMs: 250, data: [112, 105, 110, 103, 13, 10] });
    expect(interval).toBeDisabled();
    // Progress is refreshed while the job runs.
    const job = backend.repeats.get(1);
    if (job) job.sent = 7;
    await waitFor(() => expect(screen.getByRole("button", { name: "Stop (7)" })).toBeInTheDocument(), {
      timeout: 2000,
    });
    // Enter does not start a second job while one runs.
    await user.type(screen.getByLabelText("Send"), "{Enter}");
    expect(backend.written).toHaveLength(1);
    await user.click(screen.getByRole("button", { name: "Stop (7)" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Start" })).toBeEnabled());
  });

  it("reports bad intervals and failed timed sends", async () => {
    const { backend, user } = setup();
    await openPort(user);
    await user.type(screen.getByLabelText("Send"), "x");
    await user.click(screen.getByRole("checkbox", { name: "Timed" }));
    const interval = screen.getByLabelText("Interval (ms)");
    await user.clear(interval);
    await user.type(interval, "0");
    await user.click(screen.getByRole("button", { name: "Start" }));
    expect(status()).toHaveTextContent("The interval must be");
    expect(backend.repeats.size).toBe(0);

    await user.clear(interval);
    await user.type(interval, "100");
    await user.click(screen.getByRole("button", { name: "Start" }));
    await waitFor(() => expect(backend.repeats.size).toBe(1));
    const job = backend.repeats.get(1);
    if (job) {
      job.running = false;
      job.error = "session is Disconnected";
    }
    await waitFor(() => expect(status()).toHaveTextContent("session is Disconnected"), {
      timeout: 2000,
    });
    expect(screen.getByRole("button", { name: "Start" })).toBeInTheDocument();
  });

  it("recalls sent texts with the arrow keys", async () => {
    const { user } = setup();
    await openPort(user);
    const input = screen.getByLabelText("Send") as HTMLTextAreaElement;
    await user.type(input, "a{Enter}");
    await user.clear(input);
    await user.type(input, "b{Enter}");
    await user.clear(input);
    await user.type(input, "a{Enter}"); // Sent again: moves to the newest place.
    await user.clear(input);
    await user.keyboard("{ArrowUp}");
    expect(input).toHaveValue("a");
    input.setSelectionRange(0, 0);
    await user.keyboard("{ArrowUp}");
    expect(input).toHaveValue("b");
    input.setSelectionRange(0, 0);
    await user.keyboard("{ArrowUp}");
    expect(input).toHaveValue("b"); // The oldest entry stays.
    input.setSelectionRange(1, 1);
    await user.keyboard("{ArrowDown}");
    expect(input).toHaveValue("a");
    input.setSelectionRange(1, 1);
    await user.keyboard("{ArrowDown}");
    expect(input).toHaveValue("");
    // Typing leaves history browsing; ↑ in the middle of text moves the caret.
    await user.type(input, "xy");
    await user.keyboard("{ArrowUp}");
    expect(input).toHaveValue("xy");
  });

  it("saves the current log to a chosen file", async () => {
    const { backend, user } = setup();
    await openPort(user);
    await user.click(screen.getByRole("button", { name: "Save" }));
    await user.click(screen.getByRole("button", { name: "Save current log…" }));
    await waitFor(() => expect(screen.getByText("Saved 2.0 KB to /home/me/out.log")).toBeInTheDocument());
    expect(backend.exports).toEqual([[1, "/home/me/out.log", "log"]]);
    expect(backend.lastSaveName).toMatch(/^COM3_\d{8}-\d{6}\.log\|log$/);

    await user.selectOptions(screen.getByLabelText("Format"), "raw");
    backend.saveTo = null; // The user cancels the dialog.
    await user.click(screen.getByRole("button", { name: "Save current log…" }));
    await waitFor(() => expect(backend.lastSaveName).toMatch(/\.bin\|raw$/));
    expect(backend.exports).toHaveLength(1);

    backend.saveTo = "/x.bin";
    backend.failExport = "cannot write /x.bin: permission denied";
    await user.click(screen.getByRole("button", { name: "Save current log…" }));
    await waitFor(() => expect(screen.getByText(/permission denied/)).toBeInTheDocument());
  });

  it("keeps saving until stopped", async () => {
    const { backend, user } = setup();
    await openPort(user);
    await user.click(screen.getByRole("button", { name: "Save" }));
    backend.saveTo = "/logs/board.bin";
    await user.click(screen.getByRole("button", { name: "Keep saving…" }));
    await waitFor(() => expect(status()).toHaveTextContent("Saving to board.bin · 0 B"));
    expect(screen.getByLabelText("Format")).toBeDisabled();
    backend.capture = { ...backend.capture, bytes: 3 * 1024 * 1024 };
    await waitFor(() => expect(status()).toHaveTextContent("Saving to board.bin · 3.0 MB"), {
      timeout: 3000,
    });
    await user.click(screen.getByRole("button", { name: "Stop saving" }));
    await waitFor(() => expect(screen.getByText("Saved 3.0 MB to /logs/board.bin")).toBeInTheDocument());
    expect(status()).not.toHaveTextContent("Saving to");
  });

  it("reports a capture that stopped with an error", async () => {
    const { backend, user } = setup();
    await openPort(user);
    await user.click(screen.getByRole("button", { name: "Save" }));
    backend.saveTo = null;
    await user.click(screen.getByRole("button", { name: "Keep saving…" }));
    expect(status()).not.toHaveTextContent("Saving to");
    backend.saveTo = "/full/disk.log";
    await user.click(screen.getByRole("button", { name: "Keep saving…" }));
    await waitFor(() => expect(status()).toHaveTextContent("Saving to disk.log"));
    backend.capture = { ...backend.capture, active: false, error: "No space left on device" };
    await waitFor(() => expect(status()).toHaveTextContent("No space left on device"), {
      timeout: 3000,
    });
    expect(status()).not.toHaveTextContent("Saving to");
  });

  it("stops listening when closed", async () => {
    const { backend, view } = setup();
    await waitFor(() => expect(backend.signalListeners.size).toBe(1));
    expect(backend.sessionListeners.size).toBe(1);
    expect(backend.logListeners.size).toBe(1);
    view.unmount();
    expect(backend.logListeners.size).toBe(0);
    expect(backend.sessionListeners.size).toBe(0);
    expect(backend.signalListeners.size).toBe(0);
  });
});

describe("AI approvals", () => {
  it("asks about each agent action in turn", async () => {
    const { backend, user } = setup();
    await waitFor(() => expect(backend.approvalListeners.size).toBe(1));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    act(() => {
      const detail = '"AT\\r\\n" (4 bytes)';
      backend.agentAsks({ tool: "io.write", session: 1, port: "COM3", detail });
      backend.agentAsks({ tool: "session.open", session: null, port: "COM4", detail: "9600 baud" });
      backend.agentAsks({ tool: "session.close", session: 1, port: "COM3", detail: "" });
    });
    const title = "An AI agent wants to make a change";
    const dialog = () => screen.getByRole("alertdialog", { name: title });
    expect(dialog()).toHaveTextContent("io.write On COM3");
    expect(dialog()).toHaveTextContent('"AT\\r\\n" (4 bytes)');
    expect(dialog()).toHaveTextContent("2 more waiting");
    expect(within(dialog()).getByRole("button", { name: "Deny" })).toHaveFocus();
    await user.click(within(dialog()).getByRole("button", { name: "Allow for this session" }));

    await waitFor(() => expect(dialog()).toHaveTextContent("session.open Open COM4"));
    expect(dialog()).toHaveTextContent("1 more waiting");
    expect(within(dialog()).queryByRole("button", { name: "Allow for this session" })).toBeNull();
    await user.click(within(dialog()).getByRole("button", { name: "Allow" }));

    await waitFor(() => expect(dialog()).toHaveTextContent("session.close On COM3"));
    expect(dialog()).not.toHaveTextContent("waiting");
    expect(dialog().querySelector("pre")).toBeNull();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    expect(backend.answers).toEqual([
      [1, "session"],
      [2, "once"],
      [3, "deny"],
    ]);
  });

  it("shows requests that were waiting before the window loaded", async () => {
    const waiting = new FakeBackend();
    waiting.approvals = [{ id: 4, tool: "io.signals", session: 2, port: "COM5", detail: "DTR on" }];
    waiting.failAnswer = true;
    const { backend, user } = setup(waiting);
    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent("DTR on");
    await user.click(within(dialog).getByRole("button", { name: "Deny" }));
    await waitFor(() => expect(status()).toHaveTextContent("request 4 is no longer waiting"));
    expect(backend.answers).toEqual([]);
  });
});

describe("MCP panel", () => {
  async function openPanel(user: ReturnType<typeof userEvent.setup>) {
    await user.click(screen.getByRole("button", { name: "AI" }));
    return screen.getByRole("region", { name: /MCP server/ });
  }

  it("enables the server and shows the Claude Code command", async () => {
    const { backend, user, copy } = setup();
    const panel = await openPanel(user);
    expect(within(panel).getByText("Not running")).toBeInTheDocument();
    await user.click(within(panel).getByLabelText("Enable MCP server"));
    await waitFor(() =>
      expect(within(panel).getByText("Running at http://127.0.0.1:9633/mcp")).toBeInTheDocument(),
    );
    expect(backend.mcp.settings.enabled).toBe(true);
    const command =
      'claude mcp add --transport http serialbot http://127.0.0.1:9633/mcp --header "Authorization: Bearer token-0123456789abcdef"';
    expect(within(panel).getByText(command)).toBeInTheDocument();
    await user.click(within(panel).getByRole("button", { name: "Copy Claude Code command" }));
    expect(copy).toHaveBeenCalledWith(command);
    await waitFor(() => expect(within(panel).getByText("Copied")).toBeInTheDocument());

    // Claude Desktop starts the stdio bridge: `serialbot mcp`.
    await user.click(within(panel).getByRole("button", { name: "Copy Claude Desktop config" }));
    const copied: unknown[][] = copy.mock.calls;
    const config: unknown = JSON.parse(String(copied.at(-1)?.[0]));
    expect(config).toEqual({
      mcpServers: {
        serialbot: {
          command: "/Applications/SerialBot.app/Contents/MacOS/serialbot",
          args: ["mcp"],
        },
      },
    });

    expect(status()).toHaveTextContent("MCP :9633");
    await user.click(screen.getByRole("button", { name: "Terminal" }));
    expect(screen.queryByRole("region", { name: /MCP server/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Terminal" })).toHaveAttribute("aria-current", "page");
    // The status bar entry leads back to the AI page.
    await user.click(screen.getByRole("button", { name: "MCP :9633" }));
    expect(screen.getByRole("region", { name: /MCP server/ })).toBeInTheDocument();
  });

  it("saves port and token, and regenerates the token", async () => {
    const { backend, user } = setup();
    const panel = await openPanel(user);
    const port = within(panel).getByLabelText("Port");
    await user.clear(port);
    await user.type(port, "9700");
    const token = within(panel).getByLabelText("Token");
    await user.clear(token);
    await user.type(token, "my-own-token-123456");
    await user.click(within(panel).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(backend.mcp.settings).toEqual({
        enabled: false,
        port: 9700,
        token: "my-own-token-123456",
        confirm: true,
      }),
    );
    // Confirming agents' changes can be turned off.
    await user.click(within(panel).getByLabelText(/Ask before AI agents/));
    await waitFor(() => expect(backend.mcp.settings.confirm).toBe(false));

    await user.click(within(panel).getByRole("button", { name: "New token" }));
    await waitFor(() => expect(token).toHaveValue("fresh-token-0123456789"));

    await user.clear(port);
    await user.type(port, "70000");
    await user.click(within(panel).getByRole("button", { name: "Save" }));
    expect(within(panel).getByText("Port must be between 1 and 65535")).toBeInTheDocument();
    expect(backend.mcp.settings.port).toBe(9700);
  });

  it("shows errors from the backend and the clipboard", async () => {
    const copy = vi.fn(async () => {
      throw "clipboard denied";
    });
    const backend = new FakeBackend();
    backend.mcp = { ...backend.mcp, running: true, url: "http://127.0.0.1:9634/mcp", error: "port 9633 in use" };
    const { user } = setup(backend, copy);
    const panel = await openPanel(user);
    expect(within(panel).getByText("port 9633 in use")).toBeInTheDocument();
    await user.click(within(panel).getByRole("button", { name: "Copy Claude Code command" }));
    await waitFor(() => expect(within(panel).getByText("clipboard denied")).toBeInTheDocument());

    backend.failMcp = "the token must be at least 16 characters";
    await user.click(within(panel).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(status()).toHaveTextContent("at least 16 characters"));
  });
});
