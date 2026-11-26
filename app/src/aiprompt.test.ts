// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { describe, expect, it } from "vitest";
import { aiPrompt, type AiPromptTexts } from "./aiprompt";
import type { Line } from "./terminal";
import { formatTime } from "./timefmt";

const texts: AiPromptTexts = {
  intro: (port, n, from, to) => `From ${port}: ${n} lines, ${from} to ${to}.`,
  legend: "RX received, TX sent.",
  question: "Why?",
};

const line = (id: number, kind: Line["kind"], text: string): Line => ({
  id,
  kind,
  text,
  timestampUs: 1_700_000_000_000_000 + id * 1000,
});

describe("aiPrompt", () => {
  it("frames the newest lines with their time and direction", () => {
    const lines = [
      line(1, "rx", "boot"),
      line(2, "info", "Port opened"),
      line(3, "tx", "version"),
      line(4, "marker", "reset"),
      line(5, "rx", "rst:0x1"),
    ];
    const text = aiPrompt(lines, "COM3", texts, 3);
    const t = (id: number) => formatTime(1_700_000_000_000_000 + id * 1000);
    expect(text).toBe(
      [
        `From COM3: 3 lines, ${t(3)} to ${t(5)}.`,
        "RX received, TX sent.",
        "",
        "```text",
        `${t(3)} TX version`,
        "--- reset ---",
        `${t(5)} RX rst:0x1`,
        "```",
        "",
        "Why?",
        "",
      ].join("\n"),
    );
  });

  it("uses a longer fence than the log's backticks, and nothing without lines", () => {
    const text = aiPrompt([line(1, "rx", "a ```` b")], "x", texts);
    expect(text).toContain("`````text\n");
    expect(text).toContain("\n`````\n");
    expect(aiPrompt([line(1, "info", "note")], "x", texts)).toBe("");
    expect(aiPrompt([], "x", texts)).toBe("");
  });
});
