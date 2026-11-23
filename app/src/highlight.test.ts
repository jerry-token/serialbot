// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { describe, expect, it } from "vitest";
import { HIGHLIGHT_COLORS, MAX_HIGHLIGHTS, makeHighlighter, parseHighlights } from "./highlight";

describe("parseHighlights", () => {
  it("takes one keyword per line with colors in turn", () => {
    const rules = parseHighlights("ERROR\n  warn \n\nerror\r\nok");
    expect(rules).toEqual([
      { word: "ERROR", color: HIGHLIGHT_COLORS[0] },
      { word: "warn", color: HIGHLIGHT_COLORS[1] },
      { word: "ok", color: HIGHLIGHT_COLORS[2] },
    ]);
  });

  it("keeps at most MAX_HIGHLIGHTS and reuses colors", () => {
    const many = Array.from({ length: 20 }, (_, i) => `k${i}`).join("\n");
    const rules = parseHighlights(many);
    expect(rules).toHaveLength(MAX_HIGHLIGHTS);
    expect(rules[HIGHLIGHT_COLORS.length]?.color).toBe(HIGHLIGHT_COLORS[0]);
  });
});

describe("makeHighlighter", () => {
  it("marks keywords ignoring case, longest first", () => {
    const split = makeHighlighter(parseHighlights("error\nERROR 42\na.b"));
    expect(split).not.toBeNull();
    expect(split?.("x Error 42, error; a.b axb")).toEqual([
      { text: "x ", color: null },
      { text: "Error 42", color: HIGHLIGHT_COLORS[1] },
      { text: ", ", color: null },
      { text: "error", color: HIGHLIGHT_COLORS[0] },
      { text: "; ", color: null },
      { text: "a.b", color: HIGHLIGHT_COLORS[2] },
      { text: " axb", color: null },
    ]);
    expect(split?.("nothing")).toEqual([{ text: "nothing", color: null }]);
    expect(split?.("")).toEqual([]);
  });

  it("is null without keywords", () => {
    expect(makeHighlighter([])).toBeNull();
  });
});
