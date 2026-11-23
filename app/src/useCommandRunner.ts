// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { useEffect, useRef, useState } from "react";
import type { Backend } from "./api";
import { encodeInput, withEndings } from "./bytes";
import { type Command, type CommandGroup, runSequence } from "./commands";
import type { RunProgress } from "./components/CommandPanel";

/**
 * The bytes a saved command sends: parsed HEX as is, or its text with its
 * line ending, in `encoding` (UTF-8 locally, others through the backend).
 * Rejects with `invalidHex` for HEX that does not parse.
 */
export async function commandBytes(
  command: Command,
  encoding: string,
  backend: Pick<Backend, "encodeText">,
  invalidHex: string,
): Promise<Uint8Array> {
  if (command.hex) {
    try {
      return encodeInput(command.text, true, "none");
    } catch {
      throw invalidHex;
    }
  }
  if (encoding !== "utf-8") {
    return Uint8Array.from(
      await backend.encodeText(withEndings(command.text, command.ending), encoding),
    );
  }
  return encodeInput(command.text, false, command.ending);
}

interface Options {
  backend: Backend;
  encoding: string;
  invalidHex: string;
  /** Called with the error text, or null after a success. */
  onResult: (error: string | null) => void;
  errorMessage: (e: unknown) => string;
}

/**
 * Sending saved commands and running groups, shared by the main window and
 * the command window. One group runs at a time; it keeps the session it
 * started on, and stops when the component goes away.
 */
export function useCommandRunner({
  backend,
  encoding,
  invalidHex,
  onResult,
  errorMessage,
}: Options) {
  const [run, setRun] = useState<RunProgress | null>(null);
  const runStop = useRef<AbortController | null>(null);
  useEffect(() => () => runStop.current?.abort(), []);

  const bytes = (c: Command) => commandBytes(c, encoding, backend, invalidHex);

  const send = async (sessionId: number, command: Command) => {
    try {
      await backend.writeData(sessionId, await bytes(command));
      onResult(null);
    } catch (e) {
      onResult(errorMessage(e));
    }
  };

  const runGroup = async (sessionId: number, group: CommandGroup) => {
    if (runStop.current) return;
    const stop = new AbortController();
    runStop.current = stop;
    const total = group.commands.length;
    try {
      await runSequence(
        group.commands,
        async (c) => {
          await backend.writeData(sessionId, await bytes(c));
        },
        stop.signal,
        (i) => setRun({ groupId: group.id, index: i + 1, total }),
      );
      onResult(null);
    } catch (e) {
      onResult(errorMessage(e));
    } finally {
      runStop.current = null;
      setRun(null);
    }
  };

  return { run, send, runGroup, stop: () => runStop.current?.abort() };
}
