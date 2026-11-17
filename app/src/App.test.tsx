import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";
import { App } from "./App";
import type {
  Backend,
  LogChunk,
  LogEntry,
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

/** In-memory backend: one log per session, like the Rust side. */
class FakeBackend implements Backend {
  ports = PORTS;
  opened: OpenRequest[] = [];
  written: number[][] = [];
  closed: number[] = [];
  log: LogEntry[] = [];
  missed = 0;
  state: SessionStateName = "open";
  error: string | null = null;
  failOpen: string | null = null;
  failList = false;
  failWrite = false;
  failRead = false;
  failClose = false;
  private nextId = 1;

  async listPorts() {
    if (this.failList) throw "permission denied: udev";
    return this.ports;
  }

  async openSession(request: OpenRequest): Promise<SessionInfo> {
    if (this.failOpen) throw this.failOpen;
    this.opened.push(request);
    this.log = [];
    this.state = "open";
    return { id: this.nextId++, port: request.port, state: "open", error: null };
  }

  async closeSession(id: number) {
    this.closed.push(id);
    if (this.failClose) throw "no session s1";
  }

  async writeData(_id: number, data: Uint8Array) {
    if (this.failWrite) throw "session is Disconnected";
    this.written.push(Array.from(data));
    this.append("tx", Array.from(data));
    return this.log.length - 1;
  }

  async readLog(_id: number, cursor: number, maxBytes: number): Promise<LogChunk> {
    if (this.failRead) throw "read failed";
    const entries: LogEntry[] = [];
    let bytes = 0;
    for (const e of this.log.slice(cursor)) {
      if (entries.length > 0 && bytes + e.data.length > maxBytes) break;
      entries.push(e);
      bytes += e.data.length;
    }
    const missed = this.missed;
    this.missed = 0;
    return { entries, next: cursor + entries.length, missed, state: this.state, error: this.error };
  }

  receive(text: string) {
    this.append("rx", Array.from(new TextEncoder().encode(text)));
  }

  private append(kind: "rx" | "tx", data: number[]) {
    this.log.push({ seq: this.log.length, timestampUs: Date.now() * 1000, kind, data, label: null });
  }
}

function setup(backend = new FakeBackend()) {
  const user = userEvent.setup();
  render(<App backend={backend} lang="en" pollMs={5} />);
  return { backend, user };
}

const log = () => screen.getByRole("log");
const status = () => screen.getByRole("status");

async function openPort(user: ReturnType<typeof userEvent.setup>) {
  await waitFor(() => expect(screen.getByLabelText("Port")).toHaveValue("COM3"));
  await user.click(screen.getByRole("button", { name: "Open" }));
  await waitFor(() => expect(status()).toHaveTextContent("COM3 · Open"));
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
    expect(screen.getByLabelText("Port")).toBeDisabled();
    expect(screen.getByRole("button", { name: "Refresh" })).toBeDisabled();
  });

  it("shows received data with timestamps, and toggles views", async () => {
    const { backend, user } = setup();
    await openPort(user);
    act(() => backend.receive("boot ok\r\nAB"));
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

  it("sends text with the line ending, and hex", async () => {
    const { backend, user } = setup();
    await openPort(user);
    const input = screen.getByLabelText("Send");
    await user.type(input, "AT{Enter}");
    await waitFor(() => expect(backend.written).toEqual([[65, 84, 13, 10]]));
    await waitFor(() => expect(log()).toHaveTextContent("→ AT"));

    await user.selectOptions(screen.getByLabelText("Line ending"), "lf");
    await user.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(backend.written[1]).toEqual([65, 84, 10]));

    await user.click(screen.getByLabelText("Show sent"));
    await waitFor(() => expect(log()).not.toHaveTextContent("→ AT"));

    await user.click(screen.getByRole("checkbox", { name: "Hex" }));
    expect(screen.getByLabelText("Line ending")).toBeDisabled();
    await user.clear(input);
    await user.type(input, "01 ff{Enter}");
    await waitFor(() => expect(backend.written[2]).toEqual([1, 255]));

    await user.type(input, " z{Enter}");
    expect(status()).toHaveTextContent("Invalid hex");
    expect(backend.written).toHaveLength(3);
  });

  it("reports an unplugged device and closes the session", async () => {
    const { backend, user } = setup();
    await openPort(user);
    act(() => {
      backend.receive("last words\n");
      backend.state = "disconnected";
    });
    await waitFor(() => expect(status()).toHaveTextContent("Disconnected"));
    expect(log()).toHaveTextContent("last words");
    expect(log()).toHaveTextContent("! Device disconnected");
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(status()).toHaveTextContent("Closed"));
    expect(backend.closed).toEqual([1]);
    expect(screen.getByLabelText("Port")).toBeEnabled();
  });

  it("shows the failure reason of a failed session", async () => {
    const { backend, user } = setup();
    await openPort(user);
    act(() => {
      backend.state = "failed";
      backend.error = "I/O error: broken";
    });
    await waitFor(() => expect(status()).toHaveTextContent("Failed"));
    expect(log()).toHaveTextContent("! I/O error: broken");
  });

  it("reports dropped log entries", async () => {
    const { backend, user } = setup();
    await openPort(user);
    act(() => {
      backend.missed = 7;
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
});
