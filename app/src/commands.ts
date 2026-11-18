// Saved command lists: named groups of commands that can be sent one by
// one or run in order with a delay after each.

import type { LineEnding } from "./bytes";

export interface Command {
  id: string;
  name: string;
  /** Text to send, or hex digits when `hex` is set. */
  text: string;
  hex: boolean;
  /** Line ending added to text commands (ignored for hex). */
  ending: LineEnding;
  /** Wait after sending, before the next command of a run (ms, 0..86400000). */
  delayMs: number;
}

export interface CommandGroup {
  id: string;
  name: string;
  commands: Command[];
}

/** Key in the webview's local storage. */
export const STORAGE_KEY = "serialbot.commands.v1";

const ENDINGS: readonly LineEnding[] = ["none", "lf", "crlf", "cr"];
const MAX_DELAY_MS = 86_400_000;

let counter = 0;

/** A new id, unique within this app run and very likely across runs. */
export function newId(): string {
  counter += 1;
  return `${Date.now().toString(36)}-${counter.toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

/** A new empty command with the given line ending. */
export function newCommand(name: string, ending: LineEnding): Command {
  return { id: newId(), name, text: "", hex: false, ending, delayMs: 100 };
}

/** A new empty group. */
export function newGroup(name: string): CommandGroup {
  return { id: newId(), name, commands: [] };
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function parseCommand(v: unknown): Command | null {
  if (!isRecord(v)) return null;
  const { id, name, text, hex, ending, delayMs } = v;
  if (typeof name !== "string" || typeof text !== "string") return null;
  const delay = typeof delayMs === "number" && Number.isFinite(delayMs) ? Math.round(delayMs) : 0;
  return {
    id: typeof id === "string" && id !== "" ? id : newId(),
    name,
    text,
    hex: hex === true,
    ending: ENDINGS.includes(ending as LineEnding) ? (ending as LineEnding) : "none",
    delayMs: Math.min(Math.max(delay, 0), MAX_DELAY_MS),
  };
}

/**
 * Groups from stored or imported JSON. Missing or odd fields get safe
 * defaults (no line ending, no delay, a fresh id); entries that are not
 * commands or groups at all are skipped. Returns null unless `v` is a list.
 */
export function parseGroups(v: unknown): CommandGroup[] | null {
  if (!Array.isArray(v)) return null;
  const groups: CommandGroup[] = [];
  for (const g of v) {
    if (!isRecord(g) || typeof g.name !== "string" || !Array.isArray(g.commands)) continue;
    groups.push({
      id: typeof g.id === "string" && g.id !== "" ? g.id : newId(),
      name: g.name,
      commands: g.commands.map(parseCommand).filter((c): c is Command => c !== null),
    });
  }
  return groups;
}

/**
 * The saved groups, or one empty group named `fallbackName` when nothing
 * (valid) is stored or storage is unavailable.
 */
export function loadGroups(storage: Storage | null, fallbackName: string): CommandGroup[] {
  let groups: CommandGroup[] | null = null;
  try {
    const raw = storage?.getItem(STORAGE_KEY);
    if (raw) groups = parseGroups(JSON.parse(raw));
  } catch {
    groups = null;
  }
  return groups && groups.length > 0 ? groups : [newGroup(fallbackName)];
}

/** Saves the groups; returns false if storage is unavailable or full. */
export function saveGroups(storage: Storage | null, groups: CommandGroup[]): boolean {
  if (!storage) return false;
  try {
    storage.setItem(STORAGE_KEY, JSON.stringify(groups));
    return true;
  } catch {
    return false;
  }
}

/** The webview's local storage, or null where it is not available. */
export function browserStorage(): Storage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

/** Resolves after `ms`, or as soon as `signal` aborts. */
export function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (signal.aborted || ms <= 0) {
      resolve();
      return;
    }
    const done = () => {
      clearTimeout(timer);
      signal.removeEventListener("abort", done);
      resolve();
    };
    const timer = setTimeout(done, ms);
    signal.addEventListener("abort", done);
  });
}

/**
 * Sends `commands` in order, waiting each one's delay before the next (not
 * after the last). `onStep(i)` is called before command `i` is sent.
 * Resolves "done" after the last command, or "stopped" once `signal`
 * aborts (the command being sent still completes). A failed send rejects
 * and ends the run.
 */
export async function runSequence(
  commands: readonly Command[],
  send: (command: Command) => Promise<void>,
  signal: AbortSignal,
  onStep: (index: number) => void = () => {},
  wait: (ms: number, signal: AbortSignal) => Promise<void> = sleep,
): Promise<"done" | "stopped"> {
  for (const [i, command] of commands.entries()) {
    if (signal.aborted) return "stopped";
    onStep(i);
    await send(command);
    if (i < commands.length - 1) await wait(command.delayMs, signal);
  }
  return signal.aborted ? "stopped" : "done";
}
