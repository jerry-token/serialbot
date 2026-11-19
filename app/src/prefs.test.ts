import { describe, expect, it } from "vitest";
import {
  DEFAULT_PREFS,
  applyAppearance,
  loadPrefs,
  normalize,
  resolveLanguage,
  resolveTheme,
  savePrefs,
} from "./prefs";

function memory(): Storage {
  const data = new Map<string, string>();
  return {
    get length() {
      return data.size;
    },
    clear: () => data.clear(),
    getItem: (k) => data.get(k) ?? null,
    key: (i) => [...data.keys()][i] ?? null,
    removeItem: (k) => void data.delete(k),
    setItem: (k, v) => void data.set(k, v),
  };
}

describe("preferences", () => {
  it("round-trip through storage", () => {
    const storage = memory();
    expect(loadPrefs(storage)).toEqual(DEFAULT_PREFS);
    const prefs = {
      theme: "dark" as const,
      language: "zh" as const,
      fontSize: 15,
      maxLines: 200_000,
      alwaysOnTop: true,
    };
    savePrefs(storage, prefs);
    expect(loadPrefs(storage)).toEqual(prefs);
    expect(loadPrefs(null)).toEqual(DEFAULT_PREFS);
  });

  it("bring bad values into range", () => {
    expect(
      normalize({
        theme: "neon",
        language: "fr",
        fontSize: 99,
        maxLines: 12.7,
        alwaysOnTop: "yes",
      }),
    ).toEqual({
      theme: "system",
      language: "system",
      fontSize: 24,
      maxLines: 1_000,
      alwaysOnTop: false,
    });
    expect(normalize({ fontSize: Number.NaN, maxLines: 5e9 })).toMatchObject({
      fontSize: DEFAULT_PREFS.fontSize,
      maxLines: 1_000_000,
    });
    expect(normalize(null)).toEqual(DEFAULT_PREFS);
  });

  it("survive broken storage", () => {
    const storage = memory();
    storage.setItem("serialbot.preferences", "{not json");
    expect(loadPrefs(storage)).toEqual(DEFAULT_PREFS);
    const full = {
      ...memory(),
      setItem: () => {
        throw new Error("quota");
      },
    };
    expect(() => savePrefs(full, DEFAULT_PREFS)).not.toThrow();
  });

  it("resolve and apply the theme", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveLanguage("system", "zh")).toBe("zh");
    expect(resolveLanguage("en", "zh")).toBe("en");
    const root = document.createElement("div");
    applyAppearance(root, { ...DEFAULT_PREFS, theme: "dark", fontSize: 16 }, false);
    expect(root.dataset.theme).toBe("dark");
    expect(root.style.getPropertyValue("--log-font-size")).toBe("16px");
  });
});
