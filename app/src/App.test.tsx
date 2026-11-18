import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { App } from "./App";
import type {
  Backend,
  LogChunk,
  LogEntry,
  McpSettings,
  McpStatus,
  OpenRequest,
  PortInfo,
  SessionInfo,
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
    settings: { enabled: false, port: 9633, token: "token-0123456789abcdef" },
    running: false,
    url: null,
    error: null,
  };
  lines: Record<number, SignalsInfo> = {};
  failSignals = false;
  signalListeners = new Set<(s: SignalsInfo) => void>();
  sessionListeners = new Set<(s: SessionInfo[]) => void>();
  logListeners = new Set<(ids: number[]) => void>();
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

  async onLogChanged(handler: (ids: number[]) => void) {
    this.logListeners.add(handler);
    return () => this.logListeners.delete(handler);
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
    };
    return this.mcp;
  }

  async mcpRegenerateToken() {
    return this.mcpApply({ ...this.mcp.settings, token: "fresh-token-0123456789" });
  }

  /** A session opened by someone else, e.g. an MCP client. */
  addSession(port: string, alias: string | null): SessionInfo {
    const info: SessionInfo = { id: this.nextId++, alias, port, state: "open", error: null };
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

function setup(backend = new FakeBackend(), copy = vi.fn(async () => {})) {
  const user = userEvent.setup();
  const view = render(<App backend={backend} lang="en" copy={copy} />);
  return { backend, user, copy, view };
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
    expect(screen.getByLabelText("Line ending")).toBeDisabled();
    await user.clear(input);
    await user.type(input, "01 ff{Enter}");
    await waitFor(() => expect(backend.written[2]).toEqual([1, [1, 255]]));

    await user.type(input, " z{Enter}");
    expect(status()).toHaveTextContent("Invalid hex");
    expect(backend.written).toHaveLength(3);
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
      'claude mcp add --transport http serialx http://127.0.0.1:9633/mcp --header "Authorization: Bearer token-0123456789abcdef"';
    expect(within(panel).getByText(command)).toBeInTheDocument();
    await user.click(within(panel).getByRole("button", { name: "Copy" }));
    expect(copy).toHaveBeenCalledWith(command);
    await waitFor(() => expect(within(panel).getByText("Copied")).toBeInTheDocument());

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
      }),
    );

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
    await user.click(within(panel).getByRole("button", { name: "Copy" }));
    await waitFor(() => expect(within(panel).getByText("clipboard denied")).toBeInTheDocument());

    backend.failMcp = "the token must be at least 16 characters";
    await user.click(within(panel).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(status()).toHaveTextContent("at least 16 characters"));
  });
});
