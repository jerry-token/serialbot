// Turns log entries into display lines.
//
// Raw entries are kept (bounded) so switching between text and hex view
// re-renders the past instead of only new data.
//
// Text view: received bytes are decoded as UTF-8 (multi-byte characters may
// be split across chunks) and broken into lines at "\r\n", "\n" or a lone
// "\r", also when a "\r\n" is split across chunks. A line that has not
// ended yet stays open and later bytes extend it; sent data or a marker
// closes it. Each line carries the time of its first byte. Control
// characters are shown as Unicode control pictures.
//
// Hex view: one line per entry.
//
// Encoding: text is decoded with any WHATWG encoding (UTF-8 by default; GBK,
// GB18030, Big5, Shift_JIS, ...). Sent data uses the same encoding.
//
// Frame gap: when set, received data that arrives after a pause longer than
// the gap starts a new line (text view), and received entries closer than
// the gap are joined into one line (hex view), so binary protocol frames
// line up.

import type { LogEntry } from "./api";
import { toHex } from "./bytes";

export type LineKind = "rx" | "tx" | "marker" | "info";

export interface Line {
  id: number;
  kind: LineKind;
  timestampUs: number;
  text: string;
}

export interface TerminalLimits {
  /** Most lines kept for display. */
  maxLines: number;
  /** Most payload bytes of raw entries kept for re-rendering. */
  maxRawBytes: number;
}

export const DEFAULT_LIMITS: TerminalLimits = { maxLines: 5000, maxRawBytes: 4 * 1024 * 1024 };

/** Encodings offered in the UI, as WHATWG labels with display names. */
export const ENCODINGS: ReadonlyArray<{ label: string; name: string }> = [
  { label: "utf-8", name: "UTF-8" },
  { label: "gbk", name: "GBK / GB2312" },
  { label: "gb18030", name: "GB18030" },
  { label: "big5", name: "Big5" },
  { label: "shift_jis", name: "Shift_JIS" },
  { label: "euc-kr", name: "EUC-KR" },
  { label: "windows-1252", name: "ASCII / Latin-1" },
];

/** Replaces C0 controls and DEL (except tab) with visible control pictures. */
export function visible(text: string): string {
  return text.replace(/[\u0000-\u0008\u000b-\u001f\u007f]/g, (c) => {
    const code = c.charCodeAt(0);
    return code === 0x7f ? "\u2421" : String.fromCharCode(0x2400 + code);
  });
}

type RawItem = { entry: LogEntry } | { info: string; timestampUs: number };

export class Terminal {
  private raw: RawItem[] = [];
  private rawBytes = 0;
  private lines: Line[] = [];
  private nextId = 0;
  private openRx: Line | null = null;
  private openHex: Line | null = null;
  private encoding = "utf-8";
  private decoder = new TextDecoder("utf-8");
  private hex = false;
  private frameGapUs = 0;
  private lastRxUs = 0;
  private pendingCr = false;

  constructor(private readonly limits: TerminalLimits = DEFAULT_LIMITS) {}

  /** Lines to display, oldest first. */
  get view(): readonly Line[] {
    return this.lines;
  }

  get hexMode(): boolean {
    return this.hex;
  }

  get encodingLabel(): string {
    return this.encoding;
  }

  /**
   * Decodes text with `label` from now on and re-renders kept entries.
   * Throws a RangeError for labels the platform does not know.
   */
  setEncoding(label: string): void {
    new TextDecoder(label); // Validates the label before changing anything.
    if (label === this.encoding) return;
    this.encoding = label;
    this.rebuild();
  }

  /** Sets the frame gap in milliseconds (0 turns it off) and re-renders. */
  setFrameGap(ms: number): void {
    const us = Math.max(0, Math.round(ms * 1000));
    if (us === this.frameGapUs) return;
    this.frameGapUs = us;
    this.rebuild();
  }

  /** Switches between text and hex view, re-rendering kept entries. */
  setHex(hex: boolean): void {
    if (hex === this.hex) return;
    this.hex = hex;
    this.rebuild();
  }

  /** Appends entries from the backend. */
  push(entries: readonly LogEntry[]): void {
    for (const entry of entries) {
      this.keep({ entry }, entry.data.length);
      this.render({ entry });
    }
    this.trimLines();
  }

  /** Appends an informational line (errors, dropped entries, state). */
  info(text: string, timestampUs: number): void {
    const item = { info: text, timestampUs };
    this.keep(item, 0);
    this.render(item);
    this.trimLines();
  }

  /** Forgets everything shown so far. */
  clear(): void {
    this.raw = [];
    this.rawBytes = 0;
    this.resetView();
  }

  private keep(item: RawItem, bytes: number): void {
    this.raw.push(item);
    this.rawBytes += bytes;
    while (this.rawBytes > this.limits.maxRawBytes && this.raw.length > 1) {
      const dropped = this.raw.shift();
      if (dropped && "entry" in dropped) this.rawBytes -= dropped.entry.data.length;
    }
  }

  private resetView(): void {
    this.lines = [];
    this.openRx = null;
    this.openHex = null;
    this.pendingCr = false;
    this.lastRxUs = 0;
    this.decoder = new TextDecoder(this.encoding);
  }

  private rebuild(): void {
    this.resetView();
    for (const item of this.raw) this.render(item);
    this.trimLines();
  }

  private add(kind: LineKind, timestampUs: number, text: string): Line {
    const line = { id: this.nextId++, kind, timestampUs, text };
    this.lines.push(line);
    return line;
  }

  private closeRx(): void {
    // Flush a multi-byte character cut off by the end of the received data.
    const rest = visible(this.decoder.decode());
    if (this.openRx) {
      this.appendRx(rest);
    } else if (rest) {
      this.add("rx", this.lastRxUs, rest);
    }
    this.openRx = null;
    this.openHex = null;
    this.pendingCr = false;
    this.decoder = new TextDecoder(this.encoding);
  }

  /** Whether received data at `timestampUs` comes after a frame gap. */
  private afterGap(timestampUs: number): boolean {
    return this.frameGapUs > 0 && timestampUs - this.lastRxUs > this.frameGapUs;
  }

  private replaceLine(line: Line, text: string): Line {
    // Replace the object so React sees the changed line.
    const updated = { ...line, text };
    const index = this.lines.lastIndexOf(line);
    if (index >= 0) this.lines[index] = updated;
    return updated;
  }

  private appendRx(text: string): void {
    if (!this.openRx || !text) return;
    this.openRx = this.replaceLine(this.openRx, this.openRx.text + text);
  }

  private render(item: RawItem): void {
    if ("info" in item) {
      this.closeRx();
      this.add("info", item.timestampUs, item.info);
      return;
    }
    const { entry } = item;
    if (entry.kind === "marker") {
      this.closeRx();
      this.add("marker", entry.timestampUs, entry.label ?? "");
      return;
    }
    const bytes = Uint8Array.from(entry.data);
    if (this.hex) {
      this.renderHex(entry.kind, entry.timestampUs, bytes);
      return;
    }
    if (entry.kind === "tx") {
      this.closeRx();
      const text = new TextDecoder(this.encoding)
        .decode(bytes)
        .replace(/(\r\n|\r|\n)$/, "");
      for (const part of text.split(/\r\n|\r|\n/)) this.add("tx", entry.timestampUs, visible(part));
      return;
    }
    if (this.afterGap(entry.timestampUs) && this.openRx) {
      // The device paused: what follows is a new frame. The decoder keeps
      // its state, so a character split by the pause is not lost.
      this.openRx = null;
      this.pendingCr = false;
    }
    this.lastRxUs = entry.timestampUs;
    this.renderRx(this.decoder.decode(bytes, { stream: true }), entry.timestampUs);
  }

  private renderHex(kind: "rx" | "tx", timestampUs: number, bytes: Uint8Array): void {
    if (kind === "tx") {
      this.openHex = null;
      this.add("tx", timestampUs, toHex(bytes));
      return;
    }
    if (bytes.length === 0) return;
    const joins = this.frameGapUs > 0 && this.openHex !== null && !this.afterGap(timestampUs);
    this.lastRxUs = timestampUs;
    if (joins && this.openHex) {
      const sep = this.openHex.text ? " " : "";
      this.openHex = this.replaceLine(this.openHex, this.openHex.text + sep + toHex(bytes));
    } else {
      const line = this.add("rx", timestampUs, toHex(bytes));
      this.openHex = this.frameGapUs > 0 ? line : null;
    }
  }

  private renderRx(received: string, timestampUs: number): void {
    let text = received;
    // A "\r\n" split across chunks: the "\r" already ended the line.
    if (this.pendingCr) {
      this.pendingCr = false;
      if (text.startsWith("\n")) text = text.slice(1);
    }
    if (text.endsWith("\r")) this.pendingCr = true;
    const parts = text.split(/\r\n|\r|\n/);
    parts.forEach((part, i) => {
      const last = i === parts.length - 1;
      if (this.openRx) {
        this.appendRx(visible(part));
      } else if (!last || part.length > 0) {
        this.openRx = this.add("rx", timestampUs, visible(part));
      }
      if (!last) this.openRx = null;
    });
  }

  private trimLines(): void {
    const excess = this.lines.length - this.limits.maxLines;
    if (excess > 0) {
      const removed = this.lines.splice(0, excess);
      if (this.openRx && removed.includes(this.openRx)) this.openRx = null;
      if (this.openHex && removed.includes(this.openHex)) this.openHex = null;
    }
  }
}
