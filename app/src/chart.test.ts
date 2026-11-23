// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { describe, expect, it } from "vitest";
import { chartThemeFromCss } from "./chart";

describe("chartThemeFromCss", () => {
  it("takes text, grid and series colors from CSS variables, with defaults", () => {
    const el = document.createElement("div");
    el.style.setProperty("--muted", "#777");
    el.style.setProperty("--ansi-4", "#00f");
    const theme = chartThemeFromCss(el.style);
    expect(theme.text).toBe("#777");
    expect(theme.grid).toBe("#dcdfe4");
    expect(theme.series).toHaveLength(12);
    expect(theme.series[0]).toBe("#00f");
    expect(theme.series[1]).not.toBe("");
  });
});
