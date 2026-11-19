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
// ANSI colors: on by default, escape sequences in received text become
// styled parts of the line (see ansi.ts) instead of "␛[1;32m" noise.
//
// Frame gap: when set, received data that arrives after a pause longer than
// the gap starts a new line (text view), and received entries closer than
// the gap are joined into one line (hex view), so binary protocol frames
// line up.

import { AnsiParser, type Part, type Style, sameStyle } from "./ansi";
import type { LogEntry } from "./api";
import { toHex } from "./bytes";

export type LineKind = "rx" | "tx" | "marker" | "info";

export interface Line {
  /** Unique for the terminal's lifetime; a re-render gives new ids. */
  id: number;
  /**
   * Line number shown in the gutter, from 1. Trimming old lines keeps the
   * numbers of the rest; clearing or re-rendering (HEX toggle, encoding)
   * counts again from the oldest kept data.
   */
  no?: number;
  kind: LineKind;
  timestampUs: number;
  /** Plain text (escape sequences removed), used for search. */
  text: string;
  /** Styled runs of `text`, present only when some of it is styled. */
  parts?: Part[];
  /** Sent or marked by an AI agent. */
  ai?: boolean;
}

export interface TerminalLimits {
  /** Most lines kept for display. */
  maxLines: number;
  /** Most payload bytes of raw entries kept for re-rendering. */
  maxRawBytes: number;
}

/**
 * Defaults sized for long captures: the log view is virtualized, so only the
 * visible rows exist in the DOM. Kept bytes are stored compactly
 * (Uint8Array), so 8 MiB of history costs about 8 MiB.
 */
export const DEFAULT_LIMITS: TerminalLimits = { maxLines: 50_000, maxRawBytes: 8 * 1024 * 1024 };

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

/** A log entry as kept for re-rendering (bytes stored compactly). */
interface RawEntry {
  kind: LogEntry["kind"];
  timestampUs: number;
  bytes: Uint8Array;
  label: string | null;
  ai: boolean;
}

type RawItem = { entry: RawEntry } | { info: string; timestampUs: number };

export class Terminal {
  /** Kept items are raw[rawHead..]; dropping advances the head (O(1)). */
  private raw: RawItem[] = [];
  private rawHead = 0;
  private rawBytes = 0;
  private lines: Line[] = [];
  private nextId = 0;
  private lineNo = 0;
  private openRx: Line | null = null;
  private openHex: Line | null = null;
  private encoding = "utf-8";
  private decoder = new TextDecoder("utf-8");
  private hex = false;
  private frameGapUs = 0;
  private ansi = true;
  private parser = new AnsiParser();
  private lastRxUs = 0;
  private pendingCr = false;

  private limits: TerminalLimits;

  constructor(limits: TerminalLimits = DEFAULT_LIMITS) {
    this.limits = { ...limits };
  }

  /**
   * Changes how many lines are kept. A lower limit drops the oldest lines
   * at once; a higher one keeps lines that arrive from now on.
   */
  setMaxLines(maxLines: number): void {
    this.limits.maxLines = Math.max(1, Math.floor(maxLines));
    const excess = this.lines.length - this.limits.maxLines;
    if (excess > 0) this.dropOldest(excess);
  }

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

  get ansiColors(): boolean {
    return this.ansi;
  }

  /** Turns interpretation of ANSI escape sequences on or off and re-renders. */
  setAnsi(on: boolean): void {
    if (on === this.ansi) return;
    this.ansi = on;
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
    for (const e of entries) {
      const entry: RawEntry = {
        kind: e.kind,
        timestampUs: e.timestampUs,
        bytes: Uint8Array.from(e.data),
        label: e.label,
        ai: e.ai === true,
      };
      this.keep({ entry }, entry.bytes.length);
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
    this.rawHead = 0;
    this.rawBytes = 0;
    this.resetView();
  }

  private keep(item: RawItem, bytes: number): void {
    this.raw.push(item);
    this.rawBytes += bytes;
    while (this.rawBytes > this.limits.maxRawBytes && this.raw.length - this.rawHead > 1) {
      const dropped = this.raw[this.rawHead];
      this.rawHead++;
      if (dropped && "entry" in dropped) this.rawBytes -= dropped.entry.bytes.length;
    }
    // Compact once the dropped head outweighs what is kept.
    if (this.rawHead > 1024 && this.rawHead * 2 > this.raw.length) {
      this.raw = this.raw.slice(this.rawHead);
      this.rawHead = 0;
    }
  }

  private resetView(): void {
    this.lines = [];
    this.lineNo = 0;
    this.openRx = null;
    this.openHex = null;
    this.pendingCr = false;
    this.lastRxUs = 0;
    this.decoder = new TextDecoder(this.encoding);
    this.parser.reset();
  }

  private rebuild(): void {
    this.resetView();
    for (let i = this.rawHead; i < this.raw.length; i++) {
      const item = this.raw[i];
      if (item) this.render(item);
    }
    this.trimLines();
  }

  private add(kind: LineKind, timestampUs: number, text: string, style: Style | null = null): Line {
    const line: Line = { id: this.nextId++, no: ++this.lineNo, kind, timestampUs, text };
    if (style) line.parts = [{ text, style }];
    this.lines.push(line);
    return line;
  }

  private byAi(line: Line, ai: boolean): void {
    if (ai) line.ai = true;
  }

  /** Splits decoded received text into styled parts (or one plain part). */
  private styled(text: string): Part[] {
    if (this.ansi) return this.parser.feed(text);
    return text ? [{ text, style: null }] : [];
  }

  private closeRx(): void {
    // Flush a multi-byte character cut off by the end of the received data.
    for (const part of this.styled(this.decoder.decode())) {
      const text = visible(part.text);
      if (this.openRx) {
        this.appendRx(text, part.style);
      } else {
        this.openRx = this.add("rx", this.lastRxUs, text, part.style);
      }
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

  private replaceLine(line: Line, text: string, parts?: Part[]): Line {
    // Replace the object so React sees the changed line.
    const updated: Line = { ...line, text };
    if (parts) updated.parts = parts;
    const index = this.lines.lastIndexOf(line);
    if (index >= 0) this.lines[index] = updated;
    return updated;
  }

  private appendRx(text: string, style: Style | null): void {
    const line = this.openRx;
    if (!line || !text) return;
    let parts = line.parts;
    if (parts || style) {
      parts = parts ? parts.slice() : [{ text: line.text, style: null }];
      const last = parts.at(-1);
      if (last && sameStyle(last.style, style)) {
        parts[parts.length - 1] = { text: last.text + text, style: last.style };
      } else {
        parts.push({ text, style });
      }
    }
    this.openRx = this.replaceLine(line, line.text + text, parts);
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
      this.byAi(this.add("marker", entry.timestampUs, entry.label ?? ""), entry.ai);
      return;
    }
    const { bytes } = entry;
    if (this.hex) {
      this.renderHex(entry.kind, entry.timestampUs, bytes, entry.ai);
      return;
    }
    if (entry.kind === "tx") {
      this.closeRx();
      const text = new TextDecoder(this.encoding)
        .decode(bytes)
        .replace(/(\r\n|\r|\n)$/, "");
      for (const part of text.split(/\r\n|\r|\n/)) {
        this.byAi(this.add("tx", entry.timestampUs, visible(part)), entry.ai);
      }
      return;
    }
    if (this.afterGap(entry.timestampUs) && this.openRx) {
      // The device paused: what follows is a new frame. The decoder keeps
      // its state, so a character split by the pause is not lost.
      this.openRx = null;
      this.pendingCr = false;
    }
    this.lastRxUs = entry.timestampUs;
    this.renderRx(this.styled(this.decoder.decode(bytes, { stream: true })), entry.timestampUs);
  }

  private renderHex(kind: "rx" | "tx", timestampUs: number, bytes: Uint8Array, ai: boolean): void {
    if (kind === "tx") {
      this.openHex = null;
      this.byAi(this.add("tx", timestampUs, toHex(bytes)), ai);
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

  private renderRx(parts: Part[], timestampUs: number): void {
    for (const part of parts) {
      let text = part.text;
      if (text === "") continue;
      // A "\r\n" split across chunks: the "\r" already ended the line.
      if (this.pendingCr) {
        this.pendingCr = false;
        if (text.startsWith("\n")) text = text.slice(1);
      }
      if (text.endsWith("\r")) this.pendingCr = true;
      const pieces = text.split(/\r\n|\r|\n/);
      pieces.forEach((piece, i) => {
        const last = i === pieces.length - 1;
        if (this.openRx) {
          this.appendRx(visible(piece), part.style);
        } else if (!last || piece.length > 0) {
          this.openRx = this.add("rx", timestampUs, visible(piece), part.style);
        }
        if (!last) this.openRx = null;
      });
    }
  }

  /**
   * Keeps at most `maxLines` lines. Large views are trimmed in batches of a
   * tenth of the limit, so a full view is not shifted on every update.
   */
  private trimLines(): void {
    const slack = Math.floor(this.limits.maxLines / 10);
    if (this.lines.length <= this.limits.maxLines + slack) return;
    const excess = this.lines.length - this.limits.maxLines;
    if (excess > 0) this.dropOldest(excess);
  }

  private dropOldest(count: number): void {
    const removed = this.lines.splice(0, count);
    if (this.openRx && removed.includes(this.openRx)) this.openRx = null;
    if (this.openHex && removed.includes(this.openHex)) this.openHex = null;
  }
}
