// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
// Numbers in received text, as time series for the Plot view.

/** CSI, OSC and two-byte escape sequences, removed before parsing. */
// biome-ignore lint/suspicious/noControlCharactersInRegex: escape sequences start with ESC.
const ANSI = /\u001b(?:\[[0-?]*[ -/]*[@-~]|\][^\u0007\u001b]*(?:\u0007|\u001b\\)|[@-_])/g;

/** One parsed sample line: series name → value. */
export type Sample = Map<string, number>;

const NUMBER = String.raw`[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?`;
const PAIR = new RegExp(String.raw`([A-Za-z_][\w.\[\]]*)\s*[=:]\s*(${NUMBER})(?![\w.])`, "g");
const BARE_LINE = new RegExp(String.raw`^\s*${NUMBER}(?:\s*[,;\s]\s*${NUMBER})*\s*[,;]?\s*$`);
const BARE = new RegExp(NUMBER, "g");
const FIREWATER = new RegExp(
  String.raw`^\s*[A-Za-z_][\w.\[\]]*\s*:\s*${NUMBER}(?:\s*,\s*${NUMBER})+\s*,?\s*$`,
);

/** Every number in `text` as `ch1`, `ch2`, … (counted even when skipped). */
function channels(text: string): Sample {
  const sample: Sample = new Map();
  let i = 1;
  for (const m of text.matchAll(BARE)) {
    const value = Number(m[0]);
    if (Number.isFinite(value)) sample.set(`ch${i}`, value);
    i++;
  }
  return sample;
}

/**
 * The numbers in one line of received text:
 * - a VOFA+ FireWater line, `prefix:1.5,2,3`: `ch1`, `ch2`, … whatever
 *   the prefix (one value after the prefix is a pair instead);
 * - `name=value` or `name: value` pairs anywhere in the line, named after
 *   the key (`temp=23.6 hum=41.2`, `x: 1, y: -2`);
 * - otherwise, a line of nothing but numbers separated by commas,
 *   semicolons or spaces, as `ch1`, `ch2`, … (CSV-like output);
 * - anything else gives an empty sample.
 * ANSI escapes are ignored; a key seen twice keeps its last value.
 */
export function parseLine(line: string): Sample {
  const text = line.replace(ANSI, "");
  if (FIREWATER.test(text)) return channels(text.slice(text.indexOf(":") + 1));
  const sample: Sample = new Map();
  for (const m of text.matchAll(PAIR)) {
    const value = Number(m[2]);
    if (m[1] && Number.isFinite(value)) sample.set(m[1], value);
  }
  if (sample.size > 0 || !BARE_LINE.test(text)) return sample;
  return channels(text);
}

/** How received bytes become samples: text lines, or VOFA+ JustFloat frames. */
export type PlotFormat = "text" | "justfloat";

/** The tail that ends every JustFloat frame (`+inf` as a float32). */
export const JUSTFLOAT_TAIL = [0x00, 0x00, 0x80, 0x7f] as const;

/** The most values a JustFloat frame may carry; longer ones are dropped. */
export const MAX_CHANNELS = 64;

/**
 * The values of one JustFloat frame body (the bytes before the tail):
 * little-endian float32s as `ch1`, `ch2`, …; NaN and infinities are left
 * out. A body that is empty, not a multiple of four bytes long, or longer
 * than MAX_CHANNELS values gives null (the frame is dropped).
 */
export function parseJustFloat(body: Uint8Array): Sample | null {
  if (body.length === 0 || body.length % 4 !== 0 || body.length / 4 > MAX_CHANNELS) return null;
  const view = new DataView(body.buffer, body.byteOffset, body.byteLength);
  const sample: Sample = new Map();
  for (let i = 0; i < body.length / 4; i++) {
    const value = view.getFloat32(i * 4, true);
    if (Number.isFinite(value)) sample.set(`ch${i + 1}`, value);
  }
  return sample;
}

/** Where the JustFloat tail starts in `bytes`, or -1. */
function indexOfTail(bytes: Uint8Array): number {
  for (let i = 0; i + 3 < bytes.length; i++) {
    if (
      bytes[i] === JUSTFLOAT_TAIL[0] &&
      bytes[i + 1] === JUSTFLOAT_TAIL[1] &&
      bytes[i + 2] === JUSTFLOAT_TAIL[2] &&
      bytes[i + 3] === JUSTFLOAT_TAIL[3]
    ) {
      return i;
    }
  }
  return -1;
}

/** Most series kept; later names are ignored. */
export const MAX_SERIES = 12;

/**
 * Time series built from received bytes: splits them into lines (CR, LF or
 * CRLF) and parses each completed line, or, in the "justfloat" format,
 * reads each whole JustFloat frame; keeps up to `capacity` points per
 * series (oldest dropped). Every series shares the time axis; a series
 * missing from a sample has a gap (null) there.
 */
export class PlotData {
  /** Timestamps in seconds, oldest first. */
  times: number[] = [];
  /** Series name → values aligned with `times` (null where absent). */
  series = new Map<string, Array<number | null>>();
  private pending = "";
  private pendingUs = 0;
  private decoder = new TextDecoder();
  private frame: Uint8Array = new Uint8Array(0);
  private frameUs = 0;
  /** Changes whenever points are added or the data is cleared. */
  version = 0;
  /** How bytes are read; change it with `setFormat`. */
  format: PlotFormat = "text";

  constructor(private readonly capacity = 10_000) {}

  /**
   * Reads bytes as `format` from now on: clears every point, then reads
   * `history` (earlier received chunks) again in the new format.
   */
  setFormat(
    format: PlotFormat,
    history: ReadonlyArray<{ bytes: Uint8Array; timestampUs: number }> = [],
  ): void {
    this.format = format;
    this.clear();
    for (const chunk of history) this.push(chunk.bytes, chunk.timestampUs);
  }

  /** Adds received bytes that arrived at `timestampUs`. */
  push(bytes: Uint8Array, timestampUs: number): void {
    if (this.format === "justfloat") {
      this.pushFrames(bytes, timestampUs);
      return;
    }
    const text = this.pending + this.decoder.decode(bytes, { stream: true });
    if (this.pending === "") this.pendingUs = timestampUs;
    const parts = text.split(/\r\n|\r|\n/);
    this.pending = parts.pop() ?? "";
    for (const line of parts) {
      this.add(parseLine(line), this.pendingUs);
      this.pendingUs = timestampUs;
    }
    if (this.pending === "") this.pendingUs = timestampUs;
  }

  private pushFrames(bytes: Uint8Array, timestampUs: number): void {
    if (this.frame.length === 0) this.frameUs = timestampUs;
    const joined = new Uint8Array(this.frame.length + bytes.length);
    joined.set(this.frame);
    joined.set(bytes, this.frame.length);
    let rest = joined;
    for (let end = indexOfTail(rest); end >= 0; end = indexOfTail(rest)) {
      const sample = parseJustFloat(rest.subarray(0, end));
      if (sample) this.add(sample, this.frameUs);
      rest = rest.subarray(end + JUSTFLOAT_TAIL.length);
      this.frameUs = timestampUs;
    }
    // No tail yet: keep at most one frame's worth, so noise cannot pile up.
    const keep = MAX_CHANNELS * 4 + JUSTFLOAT_TAIL.length - 1;
    this.frame = rest.slice(Math.max(0, rest.length - keep));
    if (this.frame.length === 0) this.frameUs = timestampUs;
  }

  /** Forgets every point and series. */
  clear(): void {
    this.times = [];
    this.series.clear();
    this.pending = "";
    this.frame = new Uint8Array(0);
    this.decoder = new TextDecoder();
    this.version++;
  }

  /** Adds one point of named values at `timestampUs` (decoded fields). */
  addSample(sample: Sample, timestampUs: number): void {
    this.add(sample, timestampUs);
  }

  private add(sample: Sample, timestampUs: number): void {
    if (sample.size === 0) return;
    for (const name of sample.keys()) {
      if (!this.series.has(name) && this.series.size < MAX_SERIES) {
        // A new series has gaps for the points before it appeared.
        this.series.set(
          name,
          this.times.map(() => null),
        );
      }
    }
    this.times.push(timestampUs / 1e6);
    for (const [name, values] of this.series) values.push(sample.get(name) ?? null);
    if (this.times.length > this.capacity) {
      const drop = this.times.length - this.capacity;
      this.times.splice(0, drop);
      for (const values of this.series.values()) values.splice(0, drop);
    }
    this.version++;
  }
}
