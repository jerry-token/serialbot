import { act, render, screen, waitFor, within } from "@testing-library/react";
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

  setState(id: number, state: SessionStateName, error: string | null = null) {
    const session = this.sessions.get(id);
    if (!session) return;
    session.info = { ...session.info, state, error };
    this.notify([id]);
  }

  private append(id: number, kind: "rx" | "tx", data: number[]) {
    const session = this.sessions.get(id);
    if (!session) return;
    session.log.push({
      seq: session.log.length,
      timestampUs: Date.now() * 1000,
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
    await user.selectOptions(screen.getByLabelText("Data"), "7");
    await user.selectOptions(screen.getByLabelText("Parity"), "even");
    await user.selectOptions(screen.getByLabelText("Stop"), "2");
    await user.selectOptions(screen.getByLabelText("Flow"), "hardware");
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

    await user.click(screen.getByLabelText("Timestamps"));
    expect(log().textContent).not.toMatch(/\[\d\d:/);

    await user.click(screen.getByLabelText("Hex view"));
    expect(log()).toHaveTextContent("62 6f 6f 74 20 6f 6b 0d 0a 41 42");
    await user.click(screen.getByLabelText("Hex view"));
    expect(log()).toHaveTextContent("boot ok");

    await user.click(screen.getByLabelText("Auto-scroll"));
    await user.click(screen.getByRole("button", { name: "Clear" }));
    expect(log()).toBeEmptyDOMElement();
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

    await user.click(screen.getByLabelText("Show sent"));
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

  it("stops listening when closed", async () => {
    const { backend, view } = setup();
    await waitFor(() => expect(backend.logListeners.size).toBe(1));
    expect(backend.sessionListeners.size).toBe(1);
    view.unmount();
    expect(backend.logListeners.size).toBe(0);
    expect(backend.sessionListeners.size).toBe(0);
  });
});

describe("MCP panel", () => {
  async function openPanel(user: ReturnType<typeof userEvent.setup>) {
    await user.click(screen.getByRole("button", { name: "MCP" }));
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

    await user.click(screen.getByRole("button", { name: "MCP" }));
    expect(screen.queryByRole("region", { name: /MCP server/ })).not.toBeInTheDocument();
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
