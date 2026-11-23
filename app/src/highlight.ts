// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
// Keywords the user wants to stand out in the log.

/** One keyword and the background it is marked with. */
export interface Highlight {
  word: string;
  color: string;
}

/** Translucent backgrounds that read in light and dark themes, used in turn. */
export const HIGHLIGHT_COLORS = [
  "rgb(239 68 68 / 35%)",
  "rgb(245 158 11 / 40%)",
  "rgb(34 197 94 / 35%)",
  "rgb(59 130 246 / 35%)",
  "rgb(168 85 247 / 35%)",
  "rgb(236 72 153 / 35%)",
] as const;

/** Most keywords kept. */
export const MAX_HIGHLIGHTS = 16;

/**
 * Keywords from the settings box: one per line, trimmed, blank lines and
 * case-insensitive repeats dropped, at most `MAX_HIGHLIGHTS`. Colors follow
 * `HIGHLIGHT_COLORS` in order and start over after the last.
 */
export function parseHighlights(text: string): Highlight[] {
  const seen = new Set<string>();
  const out: Highlight[] = [];
  for (const raw of text.split(/\r?\n/)) {
    const word = raw.trim();
    const key = word.toLowerCase();
    if (word === "" || seen.has(key)) continue;
    seen.add(key);
    const color = HIGHLIGHT_COLORS[out.length % HIGHLIGHT_COLORS.length] ?? HIGHLIGHT_COLORS[0];
    out.push({ word, color });
    if (out.length === MAX_HIGHLIGHTS) break;
  }
  return out;
}

/** A piece of a line; `color` is set on a keyword. */
export interface Segment {
  text: string;
  color: string | null;
}

/** Splits text into keyword and plain segments. */
export type Highlighter = (text: string) => Segment[];

/**
 * A highlighter for `rules`, or null when there are none. Matching ignores
 * case; where keywords overlap the longer one wins (so "ERROR" and
 * "ERROR 42" both work), and matches never overlap.
 */
export function makeHighlighter(rules: readonly Highlight[]): Highlighter | null {
  if (rules.length === 0) return null;
  const colors = new Map(rules.map((r) => [r.word.toLowerCase(), r.color]));
  const words = [...rules].map((r) => r.word).sort((a, b) => b.length - a.length);
  const pattern = new RegExp(words.map(escape).join("|"), "gi");
  return (text) => {
    const out: Segment[] = [];
    let last = 0;
    for (const match of text.matchAll(pattern)) {
      const start = match.index;
      if (start > last) out.push({ text: text.slice(last, start), color: null });
      out.push({ text: match[0], color: colors.get(match[0].toLowerCase()) ?? null });
      last = start + match[0].length;
    }
    if (last < text.length) out.push({ text: text.slice(last), color: null });
    return out;
  };
}

function escape(word: string): string {
  return word.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
