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

/**
 * The numbers in one line of received text:
 * - `name=value` or `name: value` pairs anywhere in the line, named after
 *   the key (`temp=23.6 hum=41.2`, `x: 1, y: -2`);
 * - otherwise, a line of nothing but numbers separated by commas,
 *   semicolons or spaces, as `ch1`, `ch2`, … (CSV-like output);
 * - anything else gives an empty sample.
 * ANSI escapes are ignored; a key seen twice keeps its last value.
 */
export function parseLine(line: string): Sample {
  const text = line.replace(ANSI, "");
  const sample: Sample = new Map();
  for (const m of text.matchAll(PAIR)) {
    const value = Number(m[2]);
    if (m[1] && Number.isFinite(value)) sample.set(m[1], value);
  }
  if (sample.size > 0 || !BARE_LINE.test(text)) return sample;
  let i = 1;
  for (const m of text.matchAll(BARE)) {
    const value = Number(m[0]);
    if (Number.isFinite(value)) sample.set(`ch${i}`, value);
    i++;
  }
  return sample;
}

/** Most series kept; later names are ignored. */
export const MAX_SERIES = 12;

/**
 * Time series built from received bytes: splits them into lines (CR, LF or
 * CRLF), parses each completed line, and keeps up to `capacity` points per
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
  /** Changes whenever points are added or the data is cleared. */
  version = 0;

  constructor(private readonly capacity = 10_000) {}

  /** Adds received bytes that arrived at `timestampUs`. */
  push(bytes: Uint8Array, timestampUs: number): void {
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

  /** Forgets every point and series. */
  clear(): void {
    this.times = [];
    this.series.clear();
    this.pending = "";
    this.decoder = new TextDecoder();
    this.version++;
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
