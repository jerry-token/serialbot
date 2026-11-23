// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
// App-wide preferences, kept in local storage.

import type { Lang } from "./i18n";

export type Theme = "system" | "light" | "dark";

/** UI language; "system" follows the operating system's language. */
export type Language = "system" | Lang;

export interface Preferences {
  theme: Theme;
  language: Language;
  /** Log font size in pixels. */
  fontSize: number;
  /** Most log lines kept per session in the window. */
  maxLines: number;
  /** Keep the window above other windows. */
  alwaysOnTop: boolean;
}

export const DEFAULT_PREFS: Preferences = {
  theme: "system",
  language: "system",
  fontSize: 12.5,
  maxLines: 50_000,
  alwaysOnTop: false,
};

export const FONT_SIZE_RANGE = [9, 24] as const;
export const MAX_LINES_RANGE = [1_000, 1_000_000] as const;

const KEY = "serialbot.preferences";

const clamp = (v: number, [lo, hi]: readonly [number, number]) => Math.min(Math.max(v, lo), hi);

/**
 * Brings stored or typed values into range: unknown themes and languages
 * become "system",
 * numbers are clamped (line counts rounded to whole lines), anything
 * missing or of the wrong type takes its default.
 */
export function normalize(value: unknown): Preferences {
  const v = (typeof value === "object" && value !== null ? value : {}) as Record<string, unknown>;
  const num = (x: unknown, fallback: number) =>
    typeof x === "number" && Number.isFinite(x) ? x : fallback;
  return {
    theme: v.theme === "light" || v.theme === "dark" ? v.theme : "system",
    language: v.language === "en" || v.language === "zh" ? v.language : "system",
    fontSize: clamp(num(v.fontSize, DEFAULT_PREFS.fontSize), FONT_SIZE_RANGE),
    maxLines: Math.round(clamp(num(v.maxLines, DEFAULT_PREFS.maxLines), MAX_LINES_RANGE)),
    alwaysOnTop: v.alwaysOnTop === true,
  };
}

/** Stored preferences, or the defaults when there are none or storage fails. */
export function loadPrefs(storage: Storage | null): Preferences {
  try {
    const raw = storage?.getItem(KEY);
    return raw ? normalize(JSON.parse(raw)) : DEFAULT_PREFS;
  } catch {
    return DEFAULT_PREFS;
  }
}

/** Stores preferences if storage allows it. */
export function savePrefs(storage: Storage | null, prefs: Preferences): void {
  try {
    storage?.setItem(KEY, JSON.stringify(prefs));
  } catch {
    // Private mode or a full quota: the preferences just are not kept.
  }
}

/** The language to show: "system" follows `systemLang`. */
export function resolveLanguage(language: Language, systemLang: Lang): Lang {
  return language === "system" ? systemLang : language;
}

/** The theme to show: "system" follows `prefersDark`. */
export function resolveTheme(theme: Theme, prefersDark: boolean): "light" | "dark" {
  return theme === "system" ? (prefersDark ? "dark" : "light") : theme;
}

/**
 * Applies theme and font size to the document: `data-theme` on the root
 * element selects the palette, `--log-font-size` sizes the log.
 */
export function applyAppearance(root: HTMLElement, prefs: Preferences, prefersDark: boolean): void {
  root.dataset.theme = resolveTheme(prefs.theme, prefersDark);
  root.style.setProperty("--log-font-size", `${prefs.fontSize}px`);
}
