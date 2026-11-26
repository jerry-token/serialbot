// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { describe, expect, it } from "vitest";
import {
  JUSTFLOAT_TAIL,
  MAX_CHANNELS,
  MAX_SERIES,
  PlotData,
  parseJustFloat,
  parseLine,
} from "./plot";

const entries = (s: Map<string, number>) => Object.fromEntries(s);

describe("parseLine", () => {
  it("reads name=value and name: value pairs", () => {
    expect(entries(parseLine("temp=23.6 hum=41.2 %"))).toEqual({ temp: 23.6, hum: 41.2 });
    expect(entries(parseLine("x: 1, y: -2.5e1, ok"))).toEqual({ x: 1, y: -25 });
    expect(entries(parseLine("[00:01] adc[0]=512 adc[1]=.5"))).toEqual({
      "adc[0]": 512,
      "adc[1]": 0.5,
    });
    // The same key twice keeps the last value; a bare number in a pair line is ignored.
    expect(entries(parseLine("v=1 v=2 3"))).toEqual({ v: 2 });
  });

  it("reads lines of bare numbers as channels", () => {
    expect(entries(parseLine("1.5, -2, 3e2"))).toEqual({ ch1: 1.5, ch2: -2, ch3: 300 });
    expect(entries(parseLine("10 20\t30;"))).toEqual({ ch1: 10, ch2: 20, ch3: 30 });
  });

  it("reads VOFA+ FireWater lines as channels, whatever the prefix", () => {
    expect(entries(parseLine("samples:1.5,-2,3e1"))).toEqual({ ch1: 1.5, ch2: -2, ch3: 30 });
    expect(entries(parseLine("  data : 4 , 5,"))).toEqual({ ch1: 4, ch2: 5 });
    // One value is still a pair, and pairs are not FireWater.
    expect(entries(parseLine("temp:21.5"))).toEqual({ temp: 21.5 });
    expect(entries(parseLine("x: 1, y: 2"))).toEqual({ x: 1, y: 2 });
    expect(parseLine("log: started, ok").size).toBe(0);
  });

  it("ignores text without numbers, versions and ANSI colors", () => {
    expect(parseLine("boot ok").size).toBe(0);
    expect(parseLine("fw 1.4.2 ready").size).toBe(0);
    expect(parseLine("v1.2.3").size).toBe(0);
    expect(entries(parseLine("\u001b[32mtemp=\u001b[1m21\u001b[0m"))).toEqual({ temp: 21 });
    expect(parseLine("").size).toBe(0);
  });
});

describe("PlotData", () => {
  const bytes = (s: string) => new TextEncoder().encode(s);

  it("splits received bytes into lines and aligns series on one time axis", () => {
    const d = new PlotData();
    d.push(bytes("temp=20\r\nboot\nte"), 1_000_000);
    d.push(bytes("mp=21 hum=40\n"), 2_000_000);
    expect(d.times).toEqual([1, 1]);
    expect(d.series.get("temp")).toEqual([20, 21]);
    // hum appeared later: a gap before it.
    expect(d.series.get("hum")).toEqual([null, 40]);
    d.push(bytes("hum=41\n"), 3_000_000);
    expect(d.series.get("temp")).toEqual([20, 21, null]);
    expect(d.times).toEqual([1, 1, 3]);
  });

  it("keeps a line's own arrival time when it spans chunks", () => {
    const d = new PlotData();
    d.push(bytes("a=1"), 5_000_000);
    d.push(bytes("\n"), 6_000_000);
    d.push(bytes("a=2\n"), 7_000_000);
    expect(d.times).toEqual([5, 7]);
  });

  it("drops the oldest points past its capacity and caps the series", () => {
    const d = new PlotData(2);
    for (let i = 0; i < 4; i++) d.push(bytes(`v=${i}\n`), i * 1e6);
    expect(d.times).toEqual([2, 3]);
    expect(d.series.get("v")).toEqual([2, 3]);
    const many = Array.from({ length: MAX_SERIES + 3 }, (_, i) => `s${i}=${i}`).join(" ");
    d.push(bytes(`${many}\n`), 9e6);
    expect(d.series.size).toBe(MAX_SERIES);
  });

  it("starts over when cleared", () => {
    const d = new PlotData();
    d.push(bytes("v=1\nv="), 1e6);
    const before = d.version;
    d.clear();
    expect(d.version).toBeGreaterThan(before);
    expect(d.times).toEqual([]);
    // The half line before the clear is gone: "2" alone is a bare channel, not v.
    d.push(bytes("2\n"), 2e6);
    expect([...d.series.keys()]).toEqual(["ch1"]);
  });
});

/** A JustFloat frame: little-endian float32 values, then the tail. */
function frame(values: number[]): Uint8Array {
  const out = new Uint8Array(values.length * 4 + 4);
  const view = new DataView(out.buffer);
  values.forEach((v, i) => view.setFloat32(i * 4, v, true));
  out.set(JUSTFLOAT_TAIL, values.length * 4);
  return out;
}

const concat = (...parts: Uint8Array[]) => {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
};

describe("parseJustFloat", () => {
  it("reads little-endian float32 channels and skips NaN", () => {
    const body = frame([1.5, Number.NaN, -3]).subarray(0, 12);
    expect(entries(parseJustFloat(body) ?? new Map())).toEqual({ ch1: 1.5, ch3: -3 });
  });

  it("drops empty, misaligned and oversized bodies", () => {
    expect(parseJustFloat(new Uint8Array(0))).toBeNull();
    expect(parseJustFloat(new Uint8Array(5))).toBeNull();
    expect(parseJustFloat(new Uint8Array((MAX_CHANNELS + 1) * 4))).toBeNull();
    expect(parseJustFloat(new Uint8Array(MAX_CHANNELS * 4))?.size).toBe(MAX_CHANNELS);
  });
});

describe("PlotData in the JustFloat format", () => {
  it("reads frames split across chunks, each at the time it started", () => {
    const d = new PlotData();
    d.setFormat("justfloat");
    const bytes = concat(frame([1, 2.5]), frame([3, 4]));
    d.push(bytes.subarray(0, 5), 1e6);
    expect(d.times).toEqual([]);
    d.push(bytes.subarray(5), 2e6);
    expect(d.times).toEqual([1, 2]);
    expect(d.series.get("ch2")).toEqual([2.5, 4]);
    // Text is not read as lines in this format.
    d.push(new TextEncoder().encode("temp=1\n"), 3e6);
    expect(d.times).toEqual([1, 2]);
  });

  it("resyncs after bad frames and does not pile up noise", () => {
    const d = new PlotData();
    d.setFormat("justfloat");
    d.push(concat(new Uint8Array([1, 2, 3]), new Uint8Array(JUSTFLOAT_TAIL), frame([7])), 1e6);
    expect(d.series.get("ch1")).toEqual([7]);
    d.push(new Uint8Array(10_000).fill(0x55), 2e6);
    // The noise kept before this frame makes it too long; the next is clean.
    d.push(frame([8]), 3e6);
    d.push(frame([9]), 4e6);
    expect(d.series.get("ch1")).toEqual([7, 9]);
  });

  it("reads the history again when the format changes", () => {
    const d = new PlotData();
    const history = [
      { bytes: new TextEncoder().encode("a=1\r\n"), timestampUs: 1e6 },
      { bytes: frame([5]), timestampUs: 2e6 },
      { bytes: frame([6]), timestampUs: 3e6 },
    ];
    for (const c of history) d.push(c.bytes, c.timestampUs);
    expect([...d.series.keys()]).toEqual(["a"]);
    d.setFormat("justfloat", history);
    expect(d.format).toBe("justfloat");
    // The text before the first frame spoils it; the next one is clean.
    expect([...d.series.entries()]).toEqual([["ch1", [6]]]);
    d.setFormat("text", history);
    expect([...d.series.keys()]).toEqual(["a"]);
  });
});
