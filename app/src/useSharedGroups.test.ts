// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { type CommandGroup, STORAGE_KEY, newCommand, newGroup, saveGroups } from "./commands";
import { useSharedGroups } from "./useSharedGroups";

function memoryStorage(): Storage {
  const items = new Map<string, string>();
  return {
    get length() {
      return items.size;
    },
    clear: () => items.clear(),
    getItem: (k) => items.get(k) ?? null,
    key: (i) => [...items.keys()][i] ?? null,
    removeItem: (k) => void items.delete(k),
    setItem: (k, v) => void items.set(k, v),
  };
}

function named(name: string, text = "AT"): CommandGroup[] {
  return [{ ...newGroup(name), id: "g", commands: [{ ...newCommand("cmd", "crlf"), id: "c", text }] }];
}

function storageEvent(newValue: string | null) {
  act(() => {
    window.dispatchEvent(new StorageEvent("storage", { key: STORAGE_KEY, newValue }));
  });
}

describe("useSharedGroups", () => {
  it("starts from storage, or one group with the fallback name", () => {
    const storage = memoryStorage();
    expect(renderHook(() => useSharedGroups(storage, "Default")).result.current[0][0]?.name).toBe(
      "Default",
    );
    saveGroups(storage, named("Board"));
    expect(renderHook(() => useSharedGroups(storage, "Default")).result.current[0][0]?.name).toBe(
      "Board",
    );
    expect(renderHook(() => useSharedGroups(null, "Default")).result.current[0][0]?.name).toBe(
      "Default",
    );
  });

  it("saves what is set", () => {
    const storage = memoryStorage();
    const { result } = renderHook(() => useSharedGroups(storage, "Default"));
    act(() => result.current[1](named("Board")));
    expect(result.current[0][0]?.name).toBe("Board");
    expect(storage.getItem(STORAGE_KEY)).toBe(JSON.stringify(named("Board")));
  });

  it("ignores late echoes of its own writes while typing", () => {
    const storage = memoryStorage();
    const { result } = renderHook(() => useSharedGroups(storage, "Default"));
    act(() => result.current[1](named("B", "A")));
    act(() => result.current[1](named("B", "A+")));
    act(() => result.current[1](named("B", "A+G")));
    storageEvent(JSON.stringify(named("B", "A")));
    storageEvent(JSON.stringify(named("B", "A+")));
    expect(result.current[0][0]?.commands[0]?.text).toBe("A+G");
  });

  it("follows the other window's edits without writing them back", () => {
    const storage = memoryStorage();
    const { result } = renderHook(() => useSharedGroups(storage, "Default"));
    act(() => result.current[1](named("Mine")));
    const written = vi.spyOn(storage, "setItem");
    storageEvent(JSON.stringify(named("Theirs")));
    expect(result.current[0][0]?.name).toBe("Theirs");
    // An edit back to a value this window once wrote is still followed.
    storageEvent(JSON.stringify(named("Mine")));
    expect(result.current[0][0]?.name).toBe("Mine");
    expect(written).not.toHaveBeenCalled();
  });

  it("reads storage when the event carries no usable groups", () => {
    const storage = memoryStorage();
    const { result } = renderHook(() => useSharedGroups(storage, "Default"));
    saveGroups(storage, named("Stored"));
    storageEvent(null);
    expect(result.current[0][0]?.name).toBe("Stored");
    saveGroups(storage, named("Again"));
    storageEvent("not json");
    expect(result.current[0][0]?.name).toBe("Again");
    storageEvent("[]");
    expect(result.current[0][0]?.name).toBe("Again");
  });

  it("ignores other keys and stops listening when unmounted", () => {
    const storage = memoryStorage();
    const { result, unmount } = renderHook(() => useSharedGroups(storage, "Default"));
    act(() => {
      window.dispatchEvent(
        new StorageEvent("storage", { key: "other", newValue: JSON.stringify(named("X")) }),
      );
    });
    expect(result.current[0][0]?.name).toBe("Default");
    unmount();
    storageEvent(JSON.stringify(named("Y")));
  });

  it("keeps a bounded memory of its writes", () => {
    const storage = memoryStorage();
    const { result } = renderHook(() => useSharedGroups(storage, "Default"));
    for (let i = 0; i < 300; i++) act(() => result.current[1](named("B", `v${i}`)));
    // The oldest writes are forgotten, so their echo now counts as an edit.
    storageEvent(JSON.stringify(named("B", "v0")));
    expect(result.current[0][0]?.commands[0]?.text).toBe("v0");
  });
});
