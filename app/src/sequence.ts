// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
// Test sequences: the script and options kept between runs of the app, a
// command group turned into a script, and the report of a sequence.

import type { SequenceStatus } from "./api";
import type { CommandGroup } from "./commands";
import type { Strings } from "./i18n";

/** What the test-sequence panel keeps in local storage. */
export interface SequenceSettings {
  script: string;
  /** How many runs; 0: until stopped. */
  runs: number;
  /** Pause between runs, in milliseconds. */
  gapMs: number;
  /** Go on after a failed run. */
  keepGoing: boolean;
}

/** Key in the webview's local storage. */
export const TEST_SEQUENCE_KEY = "serialbot.testSequence.v1";

/** The longest pause between runs the backend accepts (one hour). */
export const MAX_GAP_MS = 3_600_000;

/** A starting script that shows the steps. */
export const EXAMPLE_SCRIPT = `# One step per line; # starts a comment
timeout 2s
send AT
expect OK
wait 100ms
`;

export const DEFAULT_SEQUENCE: SequenceSettings = {
  script: EXAMPLE_SCRIPT,
  runs: 10,
  gapMs: 0,
  keepGoing: false,
};

function wholeNumber(v: unknown, max: number, fallback: number): number {
  return typeof v === "number" && Number.isInteger(v) && v >= 0 && v <= max
    ? v
    : fallback;
}

/** The saved settings, or the defaults for anything missing or invalid. */
export function loadSequence(storage: Storage | null): SequenceSettings {
  let saved: unknown = null;
  try {
    saved = JSON.parse(storage?.getItem(TEST_SEQUENCE_KEY) ?? "null");
  } catch {
    saved = null;
  }
  if (typeof saved !== "object" || saved === null) return DEFAULT_SEQUENCE;
  const s = saved as Record<string, unknown>;
  return {
    script: typeof s.script === "string" ? s.script : DEFAULT_SEQUENCE.script,
    runs: wholeNumber(s.runs, 1_000_000_000, DEFAULT_SEQUENCE.runs),
    gapMs: wholeNumber(s.gapMs, MAX_GAP_MS, DEFAULT_SEQUENCE.gapMs),
    keepGoing: s.keepGoing === true,
  };
}

/** Saves the settings; false if storage is unavailable or full. */
export function saveSequence(
  storage: Storage | null,
  settings: SequenceSettings,
): boolean {
  if (!storage) return false;
  try {
    storage.setItem(TEST_SEQUENCE_KEY, JSON.stringify(settings));
    return true;
  } catch {
    return false;
  }
}

/** Text as a `send` argument: backslashes and line breaks escaped, and a
 * `#` that would start a comment. */
function escapeSend(text: string): string {
  return text
    .replace(/\\/g, "\\\\")
    .replace(/\r/g, "\\r")
    .replace(/\n/g, "\\n")
    .replace(/\t/g, "\\t")
    .replace(/#/g, "\\#");
}

/**
 * A command group as a script: each command is sent with its own line
 * ending (an `ending` line where it changes), then its delay is waited.
 * Hex commands become `hex` steps.
 */
export function groupToScript(group: CommandGroup): string {
  const lines = [`# ${group.name.replace(/\n/g, " ")}`];
  let ending: string | null = null;
  for (const c of group.commands) {
    if (c.hex) {
      lines.push(`hex ${c.text.trim()}`);
    } else {
      if (c.ending !== ending) {
        lines.push(`ending ${c.ending}`);
        ending = c.ending;
      }
      lines.push(`send ${escapeSend(c.text)}`);
    }
    if (c.delayMs > 0) lines.push(`wait ${c.delayMs}ms`);
  }
  return `${lines.join("\n")}\n`;
}

/** The average of the passed runs, in whole milliseconds. */
export function averageMs(status: SequenceStatus): number | null {
  return status.passed > 0 ? Math.round(status.passedMs / status.passed) : null;
}

/** One failure as a line of text. */
export function failureText(
  t: Strings,
  f: SequenceStatus["failures"][number],
): string {
  const reason =
    f.reason.kind === "timeout" ? t.sequenceTimeout : f.reason.message;
  return t.sequenceFailure(f.run, f.line, f.source, reason);
}

/** A plain-text report: what ran where, the tally, run times, failures
 * and the script. `started` is when the sequence was started. */
export function sequenceReport(
  t: Strings,
  status: SequenceStatus,
  port: string,
  script: string,
  started: Date,
): string {
  const average = averageMs(status);
  const lines = [
    t.reportTitle,
    "",
    `${t.reportPort}: ${port}`,
    `${t.reportStarted}: ${started.toLocaleString()}`,
    `${t.reportTook}: ${(status.elapsedMs / 1000).toFixed(1)} s`,
    `${t.sequenceTally(status.done, status.passed, status.failed)}${
      status.stopped ? ` (${t.sequenceStopped})` : ""
    }`,
  ];
  if (
    average !== null &&
    status.fastestMs !== null &&
    status.slowestMs !== null
  ) {
    lines.push(t.sequenceTimes(status.fastestMs, average, status.slowestMs));
  }
  if (status.error) lines.push(`${t.reportError}: ${status.error}`);
  if (status.failures.length > 0) {
    lines.push("", `${t.reportFailures}:`);
    for (const f of status.failures) lines.push(`  ${failureText(t, f)}`);
    const more = status.failed - status.failures.length;
    if (more > 0) lines.push(`  ${t.sequenceMoreFailures(more)}`);
  }
  lines.push("", `${t.reportScript}:`, script.trimEnd());
  return `${lines.join("\n")}\n`;
}
