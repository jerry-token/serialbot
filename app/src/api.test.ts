import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
type Handler = (event: { payload: unknown }) => void;
const listen = vi.fn(async (_name: string, _handler: Handler) => () => {});
vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, handler: Handler) => listen(name, handler),
}));
const save = vi.fn(async (_options: unknown) => "/picked.log");
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: (o: unknown) => save(o) }));

const { claudeCommand, errorMessage, formatBytes, portLabel, saveFileName, tauriBackend, utcOffsetMin } =
  await import("./api");
const { detectLang, strings } = await import("./i18n");
const { formatTime } = await import("./timefmt");

const port = {
  path: "COM3",
  kind: "usb" as const,
  vid: 0x10c4,
  pid: 0xea60,
  manufacturer: "Silicon Labs",
  product: "CP2102",
  serialNumber: null,
};

describe("tauriBackend", () => {
  beforeEach(() => {
    invoke.mockReset();
    invoke.mockResolvedValue("result");
  });

  it("maps calls to the Rust commands", async () => {
    await tauriBackend.listPorts();
    const request = {
      port: "COM3",
      baudRate: 9600,
      dataBits: 8,
      parity: "none" as const,
      stopBits: 1,
      flowControl: "none" as const,
    };
    await tauriBackend.openSession(request);
    await tauriBackend.closeSession(1);
    await tauriBackend.writeData(1, new Uint8Array([1, 2]));
    await tauriBackend.readLog(1, 5, 100);
    await tauriBackend.listSessions();
    const settings = { enabled: true, port: 9633, token: "t", confirm: true };
    await tauriBackend.mcpStatus();
    await tauriBackend.mcpApply(settings);
    await tauriBackend.mcpRegenerateToken();
    await tauriBackend.exportLog(1, "/a.log", "log");
    await tauriBackend.startCapture(1, "/b.bin", "raw");
    await tauriBackend.stopCapture(1);
    await tauriBackend.captureStatus(1);
    await tauriBackend.startRepeat(1, new Uint8Array([7]), 100, null);
    await tauriBackend.stopRepeat(1);
    await tauriBackend.repeatStatus(1);
    await tauriBackend.encodeText("你", "gbk");
    await tauriBackend.signals(1);
    await tauriBackend.setSignals(1, { dtr: true });
    await tauriBackend.setSignals(1, { rts: false });
    await tauriBackend.runSignals(1, [{ dtr: true, rts: null, waitMs: 5 }]);
    await tauriBackend.sendBreak(1, 250);
    await tauriBackend.pendingApprovals();
    await tauriBackend.answerApproval(3, "session");
    expect(invoke.mock.calls).toEqual([
      ["list_ports"],
      ["open_session", { request }],
      ["close_session", { id: 1 }],
      ["write_data", { id: 1, data: [1, 2] }],
      ["read_log", { id: 1, cursor: 5, maxBytes: 100 }],
      ["list_sessions"],
      ["mcp_status"],
      ["mcp_apply", { settings }],
      ["mcp_regenerate_token"],
      ["export_log", { id: 1, path: "/a.log", format: "log", utcOffsetMin: utcOffsetMin() }],
      ["start_capture", { id: 1, path: "/b.bin", format: "raw", utcOffsetMin: utcOffsetMin() }],
      ["stop_capture", { id: 1 }],
      ["capture_status", { id: 1 }],
      ["start_repeat", { id: 1, data: [7], intervalMs: 100, count: null }],
      ["stop_repeat", { id: 1 }],
      ["repeat_status", { id: 1 }],
      ["encode_text", { text: "你", encoding: "gbk" }],
      ["signals", { id: 1 }],
      ["set_signals", { id: 1, dtr: true, rts: null }],
      ["set_signals", { id: 1, dtr: null, rts: false }],
      ["run_signals", { id: 1, steps: [{ dtr: true, rts: null, waitMs: 5 }] }],
      ["send_break", { id: 1, ms: 250 }],
      ["pending_approvals"],
      ["answer_approval", { id: 3, decision: "session" }],
    ]);
  });

  it("asks where to save with a filter for the format", async () => {
    expect(await tauriBackend.chooseSaveFile("a.log", "log")).toBe("/picked.log");
    await tauriBackend.chooseSaveFile("a.bin", "raw");
    expect(save.mock.calls).toEqual([
      [{ defaultPath: "a.log", filters: [{ name: "Log", extensions: ["log", "txt"] }] }],
      [{ defaultPath: "a.bin", filters: [{ name: "Raw data", extensions: ["bin", "txt"] }] }],
    ]);
  });

  it("forwards event payloads", async () => {
    const sessions = vi.fn();
    const logs = vi.fn();
    await tauriBackend.onSessionsChanged(sessions);
    await tauriBackend.onLogChanged(logs);
    const lines = vi.fn();
    await tauriBackend.onSignalsChanged(lines);
    const approvals = vi.fn();
    await tauriBackend.onApprovalsChanged(approvals);
    expect(listen.mock.calls.map((c) => c[0])).toEqual([
      "sessions-changed",
      "log-changed",
      "signals-changed",
      "approvals-changed",
    ]);
    listen.mock.calls[3]?.[1]({ payload: [{ id: 7 }] });
    expect(approvals).toHaveBeenCalledWith([{ id: 7 }]);
    listen.mock.calls[0]?.[1]({ payload: [{ id: 1 }] });
    listen.mock.calls[1]?.[1]({ payload: [1, 2] });
    listen.mock.calls[2]?.[1]({ payload: { id: 1 } });
    expect(sessions).toHaveBeenCalledWith([{ id: 1 }]);
    expect(logs).toHaveBeenCalledWith([1, 2]);
    expect(lines).toHaveBeenCalledWith({ id: 1 });
  });
});

describe("helpers", () => {
  it("names save files after the port and the time", () => {
    const at = new Date(2026, 10, 18, 9, 5, 7);
    expect(saveFileName("/dev/ttyUSB0", "log", at)).toBe("ttyUSB0_20261118-090507.log");
    expect(saveFileName("COM3", "raw", at)).toBe("COM3_20261118-090507.bin");
    expect(saveFileName("\\\\.\\COM10", "raw", at)).toBe("COM10_20261118-090507.bin");
    expect(saveFileName("/tmp/my port!", "log", at)).toBe("my_port__20261118-090507.log");
    expect(saveFileName("/", "log", at)).toBe("serial_20261118-090507.log");
  });

  it("formats sizes and the time zone", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MB");
    const date = new Date();
    expect(utcOffsetMin(date)).toBe(-date.getTimezoneOffset());
  });

  it("builds the Claude Code command", () => {
    expect(claudeCommand("http://127.0.0.1:9633/mcp", "abc")).toBe(
      'claude mcp add --transport http serialbot http://127.0.0.1:9633/mcp --header "Authorization: Bearer abc"',
    );
  });

  it("labels ports with what is known", () => {
    expect(portLabel(port)).toBe("COM3 — CP2102 (10c4:ea60)");
    expect(portLabel({ ...port, product: null })).toBe("COM3 — Silicon Labs (10c4:ea60)");
    expect(portLabel({ ...port, product: null, manufacturer: null })).toBe("COM3 (10c4:ea60)");
    expect(portLabel({ ...port, vid: null, pid: null })).toBe("COM3 — CP2102");
    expect(
      portLabel({ ...port, kind: "unknown", vid: null, pid: null, product: null, manufacturer: null }),
    ).toBe("COM3");
  });

  it("turns rejections into text", () => {
    expect(errorMessage("port is busy")).toBe("port is busy");
    expect(errorMessage(new Error("boom"))).toBe("boom");
    expect(errorMessage(42)).toBe("42");
  });

  it("picks the language from the preferred system language", () => {
    expect(detectLang(["zh-CN", "en"])).toBe("zh");
    expect(detectLang(["en-US", "zh-CN"])).toBe("en");
    expect(detectLang([])).toBe("en");
    expect(strings("zh").open).toBe("打开");
    expect(strings("en").open).toBe("Open");
    expect(strings("zh").missed(3)).toContain("3");
    expect(strings("en").missed(3)).toContain("3");
  });

  it("formats timestamps as local time with milliseconds", () => {
    const local = new Date(2026, 10, 17, 9, 5, 7, 42).getTime();
    expect(formatTime(local * 1000 + 999)).toBe("09:05:07.042");
  });
});
