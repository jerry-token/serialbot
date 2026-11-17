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
  private decoder = new TextDecoder("utf-8");
  private hex = false;
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
    this.pendingCr = false;
    this.decoder = new TextDecoder("utf-8");
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
    this.pendingCr = false;
    this.decoder = new TextDecoder("utf-8");
  }

  private appendRx(text: string): void {
    if (!this.openRx || !text) return;
    // Replace the object so React sees the changed line.
    const updated = { ...this.openRx, text: this.openRx.text + text };
    const index = this.lines.lastIndexOf(this.openRx);
    if (index >= 0) this.lines[index] = updated;
    this.openRx = updated;
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
      this.add(entry.kind, entry.timestampUs, toHex(bytes));
      return;
    }
    if (entry.kind === "tx") {
      this.closeRx();
      const text = new TextDecoder("utf-8").decode(bytes).replace(/(\r\n|\r|\n)$/, "");
      for (const part of text.split(/\r\n|\r|\n/)) this.add("tx", entry.timestampUs, visible(part));
      return;
    }
    this.lastRxUs = entry.timestampUs;
    this.renderRx(this.decoder.decode(bytes, { stream: true }), entry.timestampUs);
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
    }
  }
}
