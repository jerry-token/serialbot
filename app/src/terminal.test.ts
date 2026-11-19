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

  it("flags what an AI agent sent, in text and hex", () => {
    const t = new Terminal();
    const ai = (e: LogEntry): LogEntry => ({ ...e, ai: true });
    t.push([entry("tx", "me\n"), ai(entry("tx", "AT\n")), ai(entry("marker", "step"))]);
    t.push([entry("rx", "OK\n")]);
    const flags = () => t.view.map((l) => `${l.text}:${l.ai === true}`);
    expect(flags()).toEqual(["me:false", "AT:true", "step:true", "OK:false"]);
    t.setHex(true);
    expect(flags()).toEqual(["6d 65 0a:false", "41 54 0a:true", "step:true", "4f 4b 0a:false"]);
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

describe("Terminal encodings", () => {
  it("decodes GBK and re-renders when the encoding changes", () => {
    const t = new Terminal();
    t.push([entry("rx", [0xce, 0xc2, 0xb6, 0xc8, 0x0a]), entry("tx", [0xc4, 0xe3])]);
    expect(texts(t)[0]).not.toContain("温");
    t.setEncoding("gbk");
    expect(t.encodingLabel).toBe("gbk");
    expect(texts(t)).toEqual(["rx:温度", "tx:你"]);
    t.setEncoding("gbk");
    expect(() => t.setEncoding("klingon")).toThrow(RangeError);
    expect(t.encodingLabel).toBe("gbk");
  });

  it("keeps a GBK character split across chunks", () => {
    const t = new Terminal();
    t.setEncoding("gbk");
    t.push([entry("rx", [0xce]), entry("rx", [0xc2, 0x0a])]);
    expect(texts(t)).toEqual(["rx:温"]);
  });
});

describe("Terminal frame gap", () => {
  const at = (ms: number, data: string | number[], kind: LogEntry["kind"] = "rx") =>
    entry(kind, data, ms * 1000);

  it("starts a new text line after a pause", () => {
    const t = new Terminal();
    t.push([at(0, "AB"), at(5, "CD"), at(50, "EF"), at(52, "GH\n")]);
    expect(texts(t)).toEqual(["rx:ABCDEFGH"]);
    t.setFrameGap(20);
    expect(texts(t)).toEqual(["rx:ABCD", "rx:EFGH"]);
    expect(t.view.map((l) => l.timestampUs)).toEqual([0, 50_000]);
    t.setFrameGap(20);
    t.setFrameGap(0);
    expect(texts(t)).toEqual(["rx:ABCDEFGH"]);
  });

  it("does not lose a character cut by a pause", () => {
    const t = new Terminal();
    t.setFrameGap(10);
    const bytes = Array.from(new TextEncoder().encode("é"));
    t.push([at(0, "x"), at(100, bytes.slice(0, 1)), at(101, bytes.slice(1))]);
    expect(texts(t)).toEqual(["rx:x", "rx:é"]);
  });

  it("joins hex frames that arrive close together", () => {
    const t = new Terminal();
    t.setHex(true);
    t.push([at(0, [1, 2]), at(3, [3]), at(40, [4])]);
    expect(texts(t)).toEqual(["rx:01 02", "rx:03", "rx:04"]);
    t.setFrameGap(10);
    expect(texts(t)).toEqual(["rx:01 02 03", "rx:04"]);
    t.push([at(42, "x", "tx"), at(43, [5]), at(44, []), at(45, [6])]);
    expect(texts(t).slice(-2)).toEqual(["tx:78", "rx:05 06"]);
    t.push([entry("marker", "m"), at(46, [7])]);
    expect(texts(t).slice(-2)).toEqual(["marker:m", "rx:07"]);
  });

  it("forgets a trimmed hex frame", () => {
    const t = new Terminal({ maxLines: 1, maxRawBytes: 100 });
    t.setHex(true);
    t.setFrameGap(10);
    t.push([at(0, [1])]);
    t.info("x", 1);
    t.push([at(2, [2])]);
    expect(texts(t)).toEqual(["rx:02"]);
  });
});

describe("Terminal ANSI colors", () => {
  // The exact bytes from a Zephyr shell reported from Ubuntu testing.
  const zephyr = [
    0x0d, 0x0a, 0x1b, 0x5b, 0x31, 0x3b, 0x33, 0x32, 0x6d, 0x75, 0x61, 0x72, 0x74, 0x3a, 0x7e,
    0x24, 0x20, 0x1b, 0x5b, 0x6d, 0x0d, 0x0a, 0x1b, 0x5b, 0x31, 0x3b, 0x33, 0x32, 0x6d, 0x75,
    0x61, 0x72, 0x74, 0x3a, 0x7e, 0x24, 0x20, 0x1b, 0x5b, 0x6d,
  ];

  it("shows a colored prompt instead of escape codes", () => {
    const t = new Terminal();
    expect(t.ansiColors).toBe(true);
    t.push([entry("rx", zephyr.slice(0, 12)), entry("rx", zephyr.slice(12))]);
    expect(texts(t)).toEqual(["rx:", "rx:uart:~$ ", "rx:uart:~$ "]);
    expect(t.view[1]?.parts).toEqual([
      { text: "uart:~$ ", style: { bold: true, fg: "var(--ansi-2)" } },
    ]);
    expect(t.view[0]?.parts).toBeUndefined();
  });

  it("shows raw escape codes when turned off", () => {
    const t = new Terminal();
    t.push([entry("rx", zephyr)]);
    t.setAnsi(false);
    expect(texts(t)[1]).toBe("rx:\u241b[1;32muart:~$ \u241b[m");
    t.setAnsi(false);
    t.setAnsi(true);
    expect(texts(t)[1]).toBe("rx:uart:~$ ");
  });

  it("mixes plain and styled text on one line", () => {
    const t = new Terminal();
    t.push([entry("rx", "a\u001b[31mb"), entry("rx", "c\u001b[0md\n")]);
    expect(texts(t)).toEqual(["rx:abcd"]);
    expect(t.view[0]?.parts).toEqual([
      { text: "a", style: null },
      { text: "bc", style: { fg: "var(--ansi-1)" } },
      { text: "d", style: null },
    ]);
  });

  it("styles a flushed cut-off character and lines started after a gap", () => {
    const t = new Terminal();
    t.setFrameGap(10);
    t.push([entry("rx", "\u001b[32mok\n", 0), entry("rx", [0xe6], 100_000), entry("tx", "x")]);
    expect(t.view[1]?.parts?.[0]?.style).toEqual({ fg: "var(--ansi-2)" });
  });
});
