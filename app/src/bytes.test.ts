import { describe, expect, it } from "vitest";
import { encodeInput, parseHex, toHex, withEndings } from "./bytes";

describe("toHex", () => {
  it("formats lowercase pairs separated by spaces", () => {
    expect(toHex([0x0a, 0xff, 0])).toBe("0a ff 00");
    expect(toHex(new Uint8Array())).toBe("");
  });
});

describe("parseHex", () => {
  it("accepts separators, prefixes and either case", () => {
    for (const input of ["0A ff", "0aff", "0a:FF", "0a-ff", "0x0a,0xFF", " 0a\nff "]) {
      expect(Array.from(parseHex(input))).toEqual([0x0a, 0xff]);
    }
    expect(parseHex("")).toEqual(new Uint8Array());
  });

  it("round-trips every byte", () => {
    const all = Array.from({ length: 256 }, (_, i) => i);
    expect(Array.from(parseHex(toHex(all)))).toEqual(all);
  });

  it("rejects bad input", () => {
    expect(() => parseHex("abc")).toThrow("odd");
    expect(() => parseHex("zz")).toThrow("invalid");
  });
});

describe("encodeInput", () => {
  const text = (bytes: Uint8Array) => new TextDecoder().decode(bytes);

  it("appends the line ending to text", () => {
    expect(text(encodeInput("AT", false, "crlf"))).toBe("AT\r\n");
    expect(text(encodeInput("AT", false, "lf"))).toBe("AT\n");
    expect(text(encodeInput("AT", false, "cr"))).toBe("AT\r");
    expect(text(encodeInput("AT", false, "none"))).toBe("AT");
  });

  it("uses the line ending between lines too", () => {
    expect(text(encodeInput("a\nb", false, "crlf"))).toBe("a\r\nb\r\n");
    expect(text(encodeInput("a\r\nb\rc", false, "lf"))).toBe("a\nb\nc\n");
    expect(text(encodeInput("a\nb", false, "none"))).toBe("a\nb");
    expect(Array.from(encodeInput("01\n02", true, "crlf"))).toEqual([1, 2]);
  });

  it("does not add a second ending after a typed line break", () => {
    // Regression: Shift+Enter alone sent "\r\n\r\n" with the CRLF ending.
    expect(text(encodeInput("\n", false, "crlf"))).toBe("\r\n");
    expect(text(encodeInput("\n", false, "cr"))).toBe("\r");
    expect(text(encodeInput("\n", false, "lf"))).toBe("\n");
    expect(text(encodeInput("\n", false, "none"))).toBe("\n");
    expect(text(encodeInput("AT\n", false, "crlf"))).toBe("AT\r\n");
    expect(text(encodeInput("AT\r\n", false, "lf"))).toBe("AT\n");
    // Each typed line break is still one line end.
    expect(text(encodeInput("a\n\n", false, "crlf"))).toBe("a\r\n\r\n");
  });

  it("exposes the text with endings for other encodings", () => {
    expect(withEndings("a\nb", "cr")).toBe("a\rb\r");
  });

  it("encodes text as UTF-8", () => {
    expect(Array.from(encodeInput("é", false, "none"))).toEqual([0xc3, 0xa9]);
  });

  it("sends hex as is, without a line ending", () => {
    expect(Array.from(encodeInput("01 02", true, "crlf"))).toEqual([1, 2]);
  });
});
