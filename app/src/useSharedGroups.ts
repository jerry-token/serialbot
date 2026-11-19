import { useCallback, useEffect, useRef, useState } from "react";
import { type CommandGroup, STORAGE_KEY, loadGroups, parseGroups, saveGroups } from "./commands";

/** Own writes remembered while waiting for their echo. */
const MAX_PENDING = 256;

/**
 * The saved command groups, shared by the main window and the command
 * window through storage. Setting them saves them; the other window's
 * saves arrive as storage events and replace them.
 *
 * WebKitGTK also delivers a window's own writes back to it as storage
 * events, late: applying such an echo while the user keeps typing would
 * undo the newest keystrokes. Each window therefore remembers the values it
 * wrote and ignores an event carrying one of them (and forgets the writes
 * before it). A value from the other window clears that memory, so a later
 * edit back to an older value is still followed. Groups that arrive from
 * storage are never written back.
 */
export function useSharedGroups(
  storage: Storage | null,
  fallbackName: string,
): [CommandGroup[], (next: CommandGroup[]) => void] {
  const [groups, setGroups] = useState<CommandGroup[]>(() => loadGroups(storage, fallbackName));
  const pending = useRef<string[]>([]);

  const change = useCallback(
    (next: CommandGroup[]) => {
      setGroups(next);
      if (!saveGroups(storage, next)) return;
      pending.current.push(JSON.stringify(next));
      if (pending.current.length > MAX_PENDING) pending.current.shift();
    },
    [storage],
  );

  useEffect(() => {
    const onStorage = (e: StorageEvent) => {
      if (e.key !== STORAGE_KEY) return;
      const own = e.newValue === null ? -1 : pending.current.indexOf(e.newValue);
      if (own >= 0) {
        pending.current.splice(0, own + 1);
        return;
      }
      pending.current = [];
      setGroups(fromEvent(e.newValue) ?? loadGroups(storage, fallbackName));
    };
    window.addEventListener("storage", onStorage);
    return () => window.removeEventListener("storage", onStorage);
  }, [storage, fallbackName]);

  return [groups, change];
}

/** Non-empty groups from an event's new value, or null to read storage instead. */
function fromEvent(raw: string | null): CommandGroup[] | null {
  if (raw === null) return null;
  try {
    const groups = parseGroups(JSON.parse(raw));
    return groups && groups.length > 0 ? groups : null;
  } catch {
    return null;
  }
}
