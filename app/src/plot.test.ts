// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { describe, expect, it } from "vitest";
import { MAX_SERIES, PlotData, parseLine } from "./plot";

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
