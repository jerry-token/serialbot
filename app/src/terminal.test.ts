import { describe, expect, it } from "vitest";
import type { LogEntry } from "./api";
import { Terminal, visible } from "./terminal";

let seq = 0;
function entry(kind: LogEntry["kind"], data: string | number[], timestampUs = 1000): LogEntry {
  const bytes = typeof data === "string" ? Array.from(new TextEncoder().encode(data)) : data;
  return { seq: seq++, timestampUs, kind, data: kind === "marker" ? [] : bytes, label: kind === "marker" ? String(data) : null };
}

const texts = (t: Terminal) => t.view.map((l) => `${l.kind}:${l.text}`);

describe("Terminal text view", () => {
  it("splits received data into lines across chunks", () => {
    const t = new Terminal();
    t.push([entry("rx", "hel", 1), entry("rx", "lo\r\nwor", 2), entry("rx", "ld\n", 3)]);
    expect(texts(t)).toEqual(["rx:hello", "rx:world"]);
    expect(t.view.map((l) => l.timestampUs)).toEqual([1, 2]);
  });

  it("treats CR, LF and CRLF as line ends, also when CRLF is split", () => {
    const t = new Terminal();
    t.push([entry("rx", "a\rb\nc\r"), entry("rx", "\nd\r\n")]);
    expect(texts(t)).toEqual(["rx:a", "rx:b", "rx:c", "rx:d"]);
  });

  it("keeps empty lines", () => {
    const t = new Terminal();
    t.push([entry("rx", "a\n\nb\n")]);
    expect(texts(t)).toEqual(["rx:a", "rx:", "rx:b"]);
  });

  it("decodes UTF-8 split across chunks", () => {
    const t = new Terminal();
    const bytes = Array.from(new TextEncoder().encode("温度"));
    t.push([entry("rx", bytes.slice(0, 2)), entry("rx", bytes.slice(2))]);
    expect(texts(t)).toEqual(["rx:温度"]);
  });

  it("flushes a cut-off character when sent data closes the line", () => {
    const t = new Terminal();
    t.push([entry("rx", "ok\n"), entry("rx", [0xe6]), entry("tx", "x")]);
    expect(texts(t)).toEqual(["rx:ok", "rx:\ufffd", "tx:x"]);
    const open = new Terminal();
    open.push([entry("rx", [0x41, 0xe6]), entry("tx", "x")]);
    expect(texts(open)).toEqual(["rx:A\ufffd", "tx:x"]);
  });

  it("shows sent data and markers on their own lines", () => {
    const t = new Terminal();
    t.push([
      entry("rx", "> "),
      entry("tx", "AT\r\n"),
      entry("rx", "OK\r\n"),
      entry("marker", "reset"),
      entry("tx", "a\nb"),
    ]);
    expect(texts(t)).toEqual(["rx:> ", "tx:AT", "rx:OK", "marker:reset", "tx:a", "tx:b"]);
  });

  it("makes control characters visible", () => {
    expect(visible("a\u0000b\u0007\tc\u007f")).toBe("a\u2400b\u2407\tc\u2421");
    const t = new Terminal();
    t.push([entry("rx", [0x01, 0x41, 0x0a])]);
    expect(texts(t)).toEqual(["rx:\u2401A"]);
  });

  it("gives a changed open line a new object", () => {
    const t = new Terminal();
    t.push([entry("rx", "a")]);
    const before = t.view[0];
    t.push([entry("rx", "b")]);
    expect(t.view[0]).not.toBe(before);
    expect(t.view[0]?.text).toBe("ab");
  });
});

describe("Terminal hex view", () => {
  it("shows one line per entry and re-renders the past on toggle", () => {
    const t = new Terminal();
    t.push([entry("rx", "AB"), entry("tx", [0xff])]);
    t.info("note", 5);
    t.setHex(true);
    expect(t.hexMode).toBe(true);
    expect(texts(t)).toEqual(["rx:41 42", "tx:ff", "info:note"]);
    t.push([entry("rx", "\n")]);
    expect(texts(t).at(-1)).toBe("rx:0a");
    t.setHex(true);
    t.setHex(false);
    expect(texts(t)).toEqual(["rx:AB", "tx:\ufffd", "info:note", "rx:"]);
  });
});

describe("Terminal limits", () => {
  it("drops the oldest lines", () => {
    const t = new Terminal({ maxLines: 2, maxRawBytes: 1000 });
    t.push([entry("rx", "1\n2\n3\n4")]);
    expect(texts(t)).toEqual(["rx:3", "rx:4"]);
    // The open line was kept, so more data extends it.
    t.push([entry("rx", "5\n")]);
    expect(texts(t)).toEqual(["rx:3", "rx:45"]);
  });

  it("forgets an open line that was trimmed away", () => {
    const t = new Terminal({ maxLines: 1, maxRawBytes: 1000 });
    t.push([entry("rx", "a")]);
    t.info("x", 1);
    t.push([entry("rx", "b")]);
    expect(texts(t)).toEqual(["rx:b"]);
  });

  it("keeps raw entries within the byte budget for re-rendering", () => {
    const t = new Terminal({ maxLines: 100, maxRawBytes: 4 });
    t.push([entry("rx", "ab\n"), entry("rx", "cd\n")]);
    t.setHex(true);
    expect(texts(t)).toEqual(["rx:63 64 0a"]);
  });

  it("clears everything", () => {
    const t = new Terminal();
    t.push([entry("rx", "half")]);
    t.clear();
    expect(t.view).toEqual([]);
    t.push([entry("rx", "new\n")]);
    expect(texts(t)).toEqual(["rx:new"]);
  });
});
