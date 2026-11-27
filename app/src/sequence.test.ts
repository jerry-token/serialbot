// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { describe, expect, it } from "vitest";
import type { SequenceStatus } from "./api";
import { newCommand, newGroup } from "./commands";
import { strings } from "./i18n";
import {
  DEFAULT_SEQUENCE,
  TEST_SEQUENCE_KEY,
  averageMs,
  groupToScript,
  loadSequence,
  saveSequence,
  sequenceReport,
} from "./sequence";

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

const status = (change: Partial<SequenceStatus>): SequenceStatus => ({
  running: false,
  name: "COM3",
  runs: 10,
  done: 10,
  passed: 8,
  failed: 2,
  step: null,
  failures: [],
  fastestMs: 100,
  slowestMs: 140,
  passedMs: 961,
  elapsedMs: 12_345,
  stopped: false,
  error: null,
  ...change,
});

describe("test sequence settings", () => {
  it("round-trip and fall back to the defaults", () => {
    const storage = memory();
    expect(loadSequence(storage)).toEqual(DEFAULT_SEQUENCE);
    expect(loadSequence(null)).toEqual(DEFAULT_SEQUENCE);
    const settings = {
      script: "send x\n",
      runs: 0,
      gapMs: 500,
      keepGoing: true,
    };
    expect(saveSequence(storage, settings)).toBe(true);
    expect(loadSequence(storage)).toEqual(settings);
    expect(saveSequence(null, settings)).toBe(false);

    storage.setItem(TEST_SEQUENCE_KEY, "{not json");
    expect(loadSequence(storage)).toEqual(DEFAULT_SEQUENCE);
    storage.setItem(
      TEST_SEQUENCE_KEY,
      JSON.stringify({ script: 5, runs: -1, gapMs: 1.5 }),
    );
    expect(loadSequence(storage)).toEqual(DEFAULT_SEQUENCE);

    const full = memory();
    full.setItem = () => {
      throw new Error("quota");
    };
    expect(saveSequence(full, settings)).toBe(false);
  });
});

describe("groupToScript", () => {
  it("sends each command with its ending and waits its delay", () => {
    const group = newGroup("Boot\ncheck");
    const at = { ...newCommand("AT", "crlf"), text: "AT # 1\\x", delayMs: 100 };
    const same = { ...newCommand("again", "crlf"), text: "AT", delayMs: 0 };
    const lf = { ...newCommand("lf", "lf"), text: "a\tb\r\n", delayMs: 0 };
    const hex = {
      ...newCommand("hex", "none"),
      text: " 01 ff ",
      hex: true,
      delayMs: 5,
    };
    group.commands = [at, same, lf, hex];
    expect(groupToScript(group)).toBe(
      [
        "# Boot check",
        "ending crlf",
        "send AT \\# 1\\\\x",
        "wait 100ms",
        "send AT",
        "ending lf",
        "send a\\tb\\r\\n",
        "hex 01 ff",
        "wait 5ms",
        "",
      ].join("\n"),
    );
  });
});

describe("sequenceReport", () => {
  it("has the tally, times, failures and the script", () => {
    const t = strings("en");
    const report = sequenceReport(
      t,
      status({
        failures: [
          { run: 3, line: 4, source: "expect OK", reason: { kind: "timeout" } },
          {
            run: 7,
            line: 2,
            source: "send AT",
            reason: { kind: "error", message: "gone" },
          },
        ],
        failed: 5,
        stopped: true,
        error: "the device went away",
      }),
      "COM3",
      "send AT\nexpect OK\n\n",
      new Date(2026, 10, 27, 9, 30),
    );
    expect(report).toContain("SerialBot test sequence report");
    expect(report).toContain("Port: COM3");
    expect(report).toContain("Took: 12.3 s");
    expect(report).toContain("Runs: 10 · passed: 8 · failed: 5 (stopped)");
    expect(report).toContain(
      "fastest 100 ms · average 120 ms · slowest 140 ms",
    );
    expect(report).toContain("Ended early: the device went away");
    expect(report).toContain(
      "Run 3, line 4 (expect OK): no reply matched in time",
    );
    expect(report).toContain("Run 7, line 2 (send AT): gone");
    expect(report).toContain("…and 3 more");
    expect(report.endsWith("Script:\nsend AT\nexpect OK\n")).toBe(true);
  });

  it("leaves out times without passed runs", () => {
    const zh = strings("zh");
    const s = status({
      passed: 0,
      failed: 1,
      done: 1,
      fastestMs: null,
      slowestMs: null,
    });
    expect(averageMs(s)).toBeNull();
    const report = sequenceReport(zh, s, "COM3", "x", new Date());
    expect(report).toContain("SerialBot 测试序列报告");
    expect(report).not.toContain("最快");
    expect(report).not.toContain("失败的轮次");
  });
});
