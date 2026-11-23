// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
// `Screen` backed by xterm.js. Glue only: it needs a real browser layout, so
// it is checked in the app rather than in jsdom (see vite.config.ts).
import { FitAddon } from "@xterm/addon-fit";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import type { Screen, ScreenOptions, ScreenTheme } from "./screen";

const ANSI_KEYS = [
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
  "brightBlack",
  "brightRed",
  "brightGreen",
  "brightYellow",
  "brightBlue",
  "brightMagenta",
  "brightCyan",
  "brightWhite",
] as const;

function xtermTheme(theme: ScreenTheme): Record<string, string> {
  const out: Record<string, string> = {
    background: theme.background,
    foreground: theme.foreground,
    cursor: theme.cursor,
    selectionBackground: theme.selectionBackground,
  };
  ANSI_KEYS.forEach((key, i) => {
    const color = theme.ansi[i];
    if (color) out[key] = color;
  });
  return out;
}

export function createXtermScreen(element: HTMLElement, options: ScreenOptions): Screen {
  const term = new Terminal({
    fontFamily: 'ui-monospace, "SF Mono", Menlo, Consolas, "Cascadia Mono", monospace',
    fontSize: options.fontSize,
    theme: xtermTheme(options.theme),
    scrollback: 10_000,
    cursorBlink: true,
    convertEol: false,
  });
  const fit = new FitAddon();
  term.loadAddon(fit);
  term.open(element);
  const screen: Screen = {
    write: (data) => term.write(data),
    reset: () => term.reset(),
    onInput: (handler) => {
      term.onData(handler);
    },
    fit: () => {
      // Fitting a hidden element would give zero rows.
      if (element.clientWidth > 0 && element.clientHeight > 0) fit.fit();
    },
    focus: () => term.focus(),
    setOptions: (next: ScreenOptions) => {
      term.options.fontSize = next.fontSize;
      term.options.theme = xtermTheme(next.theme);
    },
    dispose: () => term.dispose(),
  };
  return screen;
}
