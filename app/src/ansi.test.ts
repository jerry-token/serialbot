import { describe, expect, it } from "vitest";
import { AnsiParser, applySgr, color256, sameStyle } from "./ansi";

const ESC = "\u001b";

describe("AnsiParser", () => {
  it("turns SGR sequences into styled parts", () => {
    const p = new AnsiParser();
    expect(p.feed(`${ESC}[1;32muart:~$ ${ESC}[m\r\n`)).toEqual([
      { text: "uart:~$ ", style: { bold: true, fg: "var(--ansi-2)" } },
      { text: "\r\n", style: null },
    ]);
  });

  it("completes a sequence split across chunks", () => {
    const p = new AnsiParser();
    expect(p.feed(`a${ESC}`)).toEqual([{ text: "a", style: null }]);
    expect(p.feed("[3")).toEqual([]);
    expect(p.feed("1mred")).toEqual([{ text: "red", style: { fg: "var(--ansi-1)" } }]);
    // The style carries over to later chunks until reset.
    expect(p.feed("more")).toEqual([{ text: "more", style: { fg: "var(--ansi-1)" } }]);
    p.reset();
    expect(p.feed("plain")).toEqual([{ text: "plain", style: null }]);
  });

  it("removes cursor, erase, title and charset sequences", () => {
    const p = new AnsiParser();
    const input = `${ESC}[2K${ESC}[1G${ESC}[?25lA${ESC}]0;title\u0007B${ESC}]2;t${ESC}\\C${ESC}(BD${ESC}7E${ESC}[`;
    expect(p.feed(input)).toEqual([{ text: "ABCDE", style: null }]);
    expect(p.feed("Jx")).toEqual([{ text: "x", style: null }]);
  });

  it("keeps incomplete OSC and charset sequences for the next chunk", () => {
    const p = new AnsiParser();
    expect(p.feed(`${ESC}]0;ti`)).toEqual([]);
    expect(p.feed(`tle${ESC}`)).toEqual([]);
    expect(p.feed("\\x")).toEqual([{ text: "x", style: null }]);
    expect(p.feed(`${ESC}(`)).toEqual([]);
    expect(p.feed("By")).toEqual([{ text: "y", style: null }]);
  });

  it("drops malformed sequences and gives up on runaway ones", () => {
    const p = new AnsiParser();
    expect(p.feed(`${ESC}[12\nok`)).toEqual([{ text: "\nok", style: null }]);
    const garbage = `${ESC}[${"1".repeat(300)}`;
    expect(p.feed(garbage)).toEqual([{ text: garbage, style: null }]);
  });
});

describe("applySgr", () => {
  it("handles attributes and resets", () => {
    let s = applySgr({}, "1;2;3;4;7");
    expect(s).toEqual({ bold: true, dim: true, italic: true, underline: true, inverse: true });
    s = applySgr(s, "22;23;24;27");
    expect(s).toEqual({});
    expect(applySgr({ bold: true }, "")).toEqual({});
    expect(applySgr({ bold: true }, "0")).toEqual({});
    expect(applySgr({}, ";1")).toEqual({ bold: true });
    expect(applySgr({}, "5")).toEqual({}); // blink: ignored
  });

  it("handles 16, 256 and true colors", () => {
    expect(applySgr({}, "31;44")).toEqual({ fg: "var(--ansi-1)", bg: "var(--ansi-4)" });
    expect(applySgr({}, "91;101")).toEqual({ fg: "var(--ansi-9)", bg: "var(--ansi-9)" });
    expect(applySgr({ fg: "x", bg: "y" }, "39;49")).toEqual({});
    expect(applySgr({}, "38;5;208")).toEqual({ fg: "rgb(255, 135, 0)" });
    expect(applySgr({}, "48;5;240")).toEqual({ bg: "rgb(88, 88, 88)" });
    expect(applySgr({}, "38;2;1;2;3")).toEqual({ fg: "rgb(1, 2, 3)" });
    expect(applySgr({}, "38:2:1:2:300")).toEqual({});
    expect(applySgr({}, "38;5;999;1")).toEqual({ bold: true });
    expect(applySgr({}, "38;9")).toEqual({});
  });

  it("maps the 256-color palette", () => {
    expect(color256(3)).toBe("var(--ansi-3)");
    expect(color256(16)).toBe("rgb(0, 0, 0)");
    expect(color256(231)).toBe("rgb(255, 255, 255)");
    expect(color256(232)).toBe("rgb(8, 8, 8)");
    expect(color256(256)).toBeUndefined();
  });

  it("compares styles", () => {
    expect(sameStyle(null, null)).toBe(true);
    expect(sameStyle({ bold: true }, null)).toBe(false);
    expect(sameStyle({ bold: true }, { bold: true })).toBe(true);
    expect(sameStyle({ fg: "a" }, { fg: "b" })).toBe(false);
  });
});
