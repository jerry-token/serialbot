// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
// The terminal emulator behind terminal mode, behind a small interface so
// the UI can be tested without a real emulator (it needs a browser layout).

/** Colors of the emulator, taken from the app's theme. */
export interface ScreenTheme {
  background: string;
  foreground: string;
  cursor: string;
  selectionBackground: string;
  /** The 16 ANSI colors: black, red, green, yellow, blue, magenta, cyan, white, then bright. */
  ansi: string[];
}

export interface ScreenOptions {
  fontSize: number;
  theme: ScreenTheme;
}

/** A terminal emulator drawn into an element. */
export interface Screen {
  /** Feeds received bytes (escape sequences included). */
  write(data: Uint8Array): void;
  /** Clears the screen and its scrollback. */
  reset(): void;
  /** Calls `handler` with what the user types (keys as the device expects them). */
  onInput(handler: (data: string) => void): void;
  /** Fits rows and columns to the element's size. */
  fit(): void;
  focus(): void;
  setOptions(options: ScreenOptions): void;
  dispose(): void;
}

/** Creates a screen; may load the emulator first (it is only fetched when needed). */
export type ScreenFactory = (
  element: HTMLElement,
  options: ScreenOptions,
) => Screen | Promise<Screen>;

/** The real emulator (xterm.js), loaded on first use. */
export const loadXtermScreen: ScreenFactory = async (element, options) =>
  (await import("./xtermScreen")).createXtermScreen(element, options);

const ANSI_VARS = Array.from({ length: 16 }, (_, i) => `--ansi-${i}`);

/** The emulator's colors from the CSS variables currently in effect. */
export function themeFromCss(style: CSSStyleDeclaration): ScreenTheme {
  const v = (name: string, fallback: string) => style.getPropertyValue(name).trim() || fallback;
  return {
    background: v("--log-bg", "#ffffff"),
    foreground: v("--rx", "#1d232b"),
    cursor: v("--accent", "#1a73e8"),
    selectionBackground: v("--accent-soft", "rgba(26, 115, 232, 0.25)"),
    ansi: ANSI_VARS.map((name) => v(name, "")),
  };
}
