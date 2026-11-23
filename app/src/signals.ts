// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
// DTR/RTS sequences typed by the user, e.g. "DTR=0 RTS=1, 100, RTS=0".

/** One step: set the lines given, then wait. Mirrors the Rust `SignalStep`. */
export interface SignalStep {
  dtr: boolean | null;
  rts: boolean | null;
  waitMs: number;
}

/** Limits the backend enforces too. */
export const MAX_STEPS = 64;
export const MAX_TOTAL_MS = 10_000;

/**
 * Parses a sequence. Items are separated by commas, semicolons or line
 * breaks; an item sets lines (`DTR=1`, `rts=0`, both separated by spaces)
 * or pauses (`100` or `100ms`). A pause belongs to the step before it, so
 * `DTR=0, 100, DTR=1` sets DTR low, waits 100 ms, then sets it high.
 * Throws a message naming the first item it cannot read, or when the
 * sequence is empty or exceeds `MAX_STEPS` / `MAX_TOTAL_MS`.
 */
export function parseSequence(text: string): SignalStep[] {
  const steps: SignalStep[] = [];
  for (const raw of text.split(/[,;\n]/)) {
    const item = raw.trim();
    if (item === "") continue;
    const pause = /^(\d+)\s*(ms)?$/i.exec(item);
    if (pause) {
      const ms = Number(pause[1]);
      const last = steps.at(-1);
      if (last) last.waitMs += ms;
      else steps.push({ dtr: null, rts: null, waitMs: ms });
      continue;
    }
    const step: SignalStep = { dtr: null, rts: null, waitMs: 0 };
    for (const part of item.split(/\s+/)) {
      const set = /^(dtr|rts)\s*=\s*([01])$/i.exec(part);
      if (!set) throw `cannot read "${item}"`;
      const line = set[1]?.toLowerCase() === "dtr" ? "dtr" : "rts";
      step[line] = set[2] === "1";
    }
    steps.push(step);
  }
  if (steps.length === 0) throw "the sequence is empty";
  if (steps.length > MAX_STEPS) throw `a sequence has at most ${MAX_STEPS} steps`;
  const total = steps.reduce((sum, s) => sum + s.waitMs, 0);
  if (total > MAX_TOTAL_MS) throw `a sequence pauses at most ${MAX_TOTAL_MS} ms in all`;
  return steps;
}
