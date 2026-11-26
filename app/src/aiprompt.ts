// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import type { Line } from "./terminal";
import { formatTime } from "./timefmt";

/** The most lines "Copy for AI" takes (the newest ones). */
export const AI_MAX_LINES = 200;

/** What the prompt says around the log, in the user's language. */
export interface AiPromptTexts {
  /** The first line: where the log comes from. */
  intro: (port: string, count: number, from: string, to: string) => string;
  /** What RX and TX mean. */
  legend: string;
  /** The question after the log, for the user to edit. */
  question: string;
}

/**
 * Text to paste into an AI chat: a line naming the port, the number of
 * lines and their time range, the newest `max` received and sent lines
 * (markers as `--- label ---`, other notes left out) with their time and
 * direction in a code block, and a question to edit. The fence is longer
 * than any run of backticks in the log. With no lines it is empty.
 */
export function aiPrompt(
  lines: readonly Line[],
  port: string,
  texts: AiPromptTexts,
  max = AI_MAX_LINES,
): string {
  const kept = lines.filter((l) => l.kind !== "info").slice(-max);
  const first = kept[0];
  const last = kept.at(-1);
  if (!first || !last) return "";
  const body = kept.map((l) =>
    l.kind === "marker"
      ? `--- ${l.text} ---`
      : `${formatTime(l.timestampUs)} ${l.kind === "rx" ? "RX" : "TX"} ${l.text}`,
  );
  const longest = Math.max(
    0,
    ...body.map((b) =>
      Math.max(0, ...(b.match(/`+/g) ?? []).map((m) => m.length)),
    ),
  );
  const fence = "`".repeat(Math.max(3, longest + 1));
  return [
    texts.intro(
      port,
      kept.length,
      formatTime(first.timestampUs),
      formatTime(last.timestampUs),
    ),
    texts.legend,
    "",
    `${fence}text`,
    ...body,
    fence,
    "",
    texts.question,
    "",
  ].join("\n");
}
