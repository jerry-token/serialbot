// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { describe, expect, it } from "vitest";
import { MAX_STEPS, parseSequence } from "./signals";

describe("parseSequence", () => {
  it("reads line settings and pauses", () => {
    expect(parseSequence("DTR=0 RTS=1, 100, rts=0; 50ms\nDTR=1")).toEqual([
      { dtr: false, rts: true, waitMs: 100 },
      { dtr: null, rts: false, waitMs: 50 },
      { dtr: true, rts: null, waitMs: 0 },
    ]);
  });

  it("adds up pauses and allows one before the first change", () => {
    expect(parseSequence("20, 30 MS, DTR=1, 5, 5")).toEqual([
      { dtr: null, rts: null, waitMs: 50 },
      { dtr: true, rts: null, waitMs: 10 },
    ]);
  });

  it("rejects what it cannot read and sequences that are too long", () => {
    expect(() => parseSequence("DTR=2")).toThrow('cannot read "DTR=2"');
    expect(() => parseSequence("DTR=1 wait")).toThrow('cannot read "DTR=1 wait"');
    expect(() => parseSequence(" , ;")).toThrow("empty");
    expect(() =>
      parseSequence(
        Array(MAX_STEPS + 1)
          .fill("DTR=1")
          .join(","),
      ),
    ).toThrow("64 steps");
    expect(() => parseSequence("DTR=1, 6000, DTR=0, 5000")).toThrow("10000 ms");
  });
});
