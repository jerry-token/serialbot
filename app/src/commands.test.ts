// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  type Command,
  STORAGE_KEY,
  browserStorage,
  loadGroups,
  newCommand,
  newGroup,
  newId,
  parseGroups,
  runSequence,
  saveGroups,
  sleep,
} from "./commands";

function memoryStorage(): Storage {
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

const cmd = (name: string, delayMs = 0): Command => ({
  id: name,
  name,
  text: name,
  hex: false,
  ending: "crlf",
  delayMs,
});

afterEach(() => vi.useRealTimers());

describe("command groups", () => {
  it("creates unique ids and empty items", () => {
    const ids = new Set(Array.from({ length: 50 }, newId));
    expect(ids.size).toBe(50);
    expect(newCommand("AT", "lf")).toMatchObject({
      name: "AT",
      text: "",
      ending: "lf",
      hex: false,
    });
    expect(newGroup("Default")).toMatchObject({ name: "Default", commands: [] });
  });

  it("parses stored JSON with safe defaults and skips junk", () => {
    expect(parseGroups({})).toBeNull();
    const groups = parseGroups([
      { id: "g", name: "ESP32", commands: [{ id: "c", name: "reset", text: "AT+RST" }, 7] },
      { name: "no commands" },
      "junk",
      {
        name: "Odd",
        commands: [
          { name: "x", text: "01", hex: true, ending: "weird", delayMs: -5 },
          { name: "y", text: "", delayMs: 1e12 },
          { name: "z", text: "", delayMs: Number.NaN },
          { name: 3, text: "" },
        ],
      },
    ]);
    expect(groups).toHaveLength(2);
    expect(groups?.[0]).toEqual({
      id: "g",
      name: "ESP32",
      commands: [
        { id: "c", name: "reset", text: "AT+RST", hex: false, ending: "none", delayMs: 0 },
      ],
    });
    const odd = groups?.[1];
    expect(odd?.id).not.toBe("");
    expect(odd?.commands.map((c) => [c.hex, c.ending, c.delayMs])).toEqual([
      [true, "none", 0],
      [false, "none", 86_400_000],
      [false, "none", 0],
    ]);
  });

  it("loads what was saved, or one fallback group", () => {
    const storage = memoryStorage();
    expect(loadGroups(storage, "Default").map((g) => g.name)).toEqual(["Default"]);
    expect(loadGroups(null, "默认").map((g) => g.name)).toEqual(["默认"]);
    storage.setItem(STORAGE_KEY, "{not json");
    expect(loadGroups(storage, "Default").map((g) => g.name)).toEqual(["Default"]);
    storage.setItem(STORAGE_KEY, "[]");
    expect(loadGroups(storage, "Default").map((g) => g.name)).toEqual(["Default"]);

    const groups = [{ ...newGroup("Boot"), commands: [cmd("AT", 50)] }];
    expect(saveGroups(storage, groups)).toBe(true);
    expect(loadGroups(storage, "Default")).toEqual(groups);
  });

  it("reports storage that is missing or full", () => {
    expect(saveGroups(null, [])).toBe(false);
    const full = memoryStorage();
    full.setItem = () => {
      throw new Error("QuotaExceededError");
    };
    expect(saveGroups(full, [])).toBe(false);
    const broken = memoryStorage();
    broken.getItem = () => {
      throw new Error("SecurityError");
    };
    expect(loadGroups(broken, "Default")).toHaveLength(1);
    expect(browserStorage()).not.toBeNull();
  });
});

describe("runSequence", () => {
  it("sends in order with each delay between commands", async () => {
    const sent: string[] = [];
    const waits: number[] = [];
    const steps: number[] = [];
    const result = await runSequence(
      [cmd("a", 10), cmd("b", 20), cmd("c", 30)],
      async (c) => void sent.push(c.name),
      new AbortController().signal,
      (i) => steps.push(i),
      async (ms) => void waits.push(ms),
    );
    expect(result).toBe("done");
    expect(sent).toEqual(["a", "b", "c"]);
    expect(steps).toEqual([0, 1, 2]);
    expect(waits).toEqual([10, 20]);
  });

  it("stops when aborted, also during a wait", async () => {
    vi.useFakeTimers();
    const stop = new AbortController();
    const sent: string[] = [];
    const run = runSequence(
      [cmd("a", 1000), cmd("b", 1000)],
      async (c) => void sent.push(c.name),
      stop.signal,
    );
    await vi.advanceTimersByTimeAsync(500);
    stop.abort();
    await expect(run).resolves.toBe("stopped");
    expect(sent).toEqual(["a"]);

    const late = new AbortController();
    const one = runSequence(
      [cmd("x")],
      async () => {
        late.abort();
      },
      late.signal,
    );
    await expect(one).resolves.toBe("stopped");
    await expect(runSequence([cmd("x")], async () => {}, late.signal)).resolves.toBe("stopped");
  });

  it("ends the run when a send fails", async () => {
    const sent: string[] = [];
    const run = runSequence(
      [cmd("a"), cmd("b"), cmd("c")],
      async (c) => {
        if (c.name === "b") throw "session is Disconnected";
        sent.push(c.name);
      },
      new AbortController().signal,
    );
    await expect(run).rejects.toBe("session is Disconnected");
    expect(sent).toEqual(["a"]);
  });

  it("sleeps for the delay unless aborted", async () => {
    vi.useFakeTimers();
    let done = false;
    const signal = new AbortController().signal;
    void sleep(100, signal).then(() => (done = true));
    await vi.advanceTimersByTimeAsync(99);
    expect(done).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(done).toBe(true);
    await expect(sleep(0, signal)).resolves.toBeUndefined();
    const aborted = new AbortController();
    aborted.abort();
    await expect(sleep(1000, aborted.signal)).resolves.toBeUndefined();
  });
});
