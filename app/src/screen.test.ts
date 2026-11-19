import { describe, expect, it } from "vitest";
import { themeFromCss } from "./screen";

describe("themeFromCss", () => {
  it("takes the palette from CSS variables, with defaults", () => {
    const el = document.createElement("div");
    el.style.setProperty("--log-bg", "#000");
    el.style.setProperty("--rx", "#eee");
    el.style.setProperty("--ansi-1", "#f00");
    const theme = themeFromCss(el.style);
    expect(theme.background).toBe("#000");
    expect(theme.foreground).toBe("#eee");
    expect(theme.cursor).toBe("#1a73e8");
    expect(theme.ansi).toHaveLength(16);
    expect(theme.ansi[1]).toBe("#f00");
    expect(theme.ansi[0]).toBe("");
  });
});
