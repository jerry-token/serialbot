// Conversions between what the user types and the bytes on the wire.

export type LineEnding = "none" | "lf" | "crlf" | "cr";

const ENDINGS: Record<LineEnding, string> = { none: "", lf: "\n", crlf: "\r\n", cr: "\r" };

/** Lowercase hex pairs separated by spaces: [0x0a, 0xff] -> "0a ff". */
export function toHex(bytes: ArrayLike<number>): string {
  const parts: string[] = [];
  for (let i = 0; i < bytes.length; i++) {
    parts.push((bytes[i] ?? 0).toString(16).padStart(2, "0"));
  }
  return parts.join(" ");
}

/**
 * Parses hex digits into bytes. Whitespace, ":", "-", "," and "0x" prefixes
 * are ignored, either case is accepted. Throws on an odd number of digits or
 * any other character.
 */
export function parseHex(text: string): Uint8Array {
  const digits = text.replace(/0x/gi, "").replace(/[\s:,-]/g, "");
  if (!/^[0-9a-fA-F]*$/.test(digits)) {
    throw new Error("invalid hex");
  }
  if (digits.length % 2 !== 0) {
    throw new Error("odd number of hex digits");
  }
  const out = new Uint8Array(digits.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(digits.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

/**
 * Bytes to send for the send box: UTF-8 text with the line ending after
 * every line (line breaks inside the text become that ending too; with no
 * ending they stay "\n"), or the parsed hex (line breaks are just
 * separators and no ending is added).
 */
export function encodeInput(text: string, hex: boolean, ending: LineEnding): Uint8Array {
  if (hex) return parseHex(text);
  const end = ENDINGS[ending];
  const body = end === "" ? text : text.replace(/\r\n|\r|\n/g, end);
  return new TextEncoder().encode(body + end);
}
