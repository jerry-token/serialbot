// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { useState } from "react";
import type { Strings } from "../i18n";
import { Popover } from "./Popover";

/** Chunk size and pause used for sending a file. */
export interface FileSendOptions {
  chunk: number;
  delayMs: number;
}

export const DEFAULT_FILE_OPTIONS: FileSendOptions = { chunk: 1024, delayMs: 0 };
const MAX_CHUNK = 65_536;
const MAX_DELAY_MS = 60_000;

interface Props {
  t: Strings;
  /** Whether a file can be sent (the selected session is open and idle). */
  enabled: boolean;
  /** Asks for the file and starts sending it with these options. */
  onStart: (options: FileSendOptions) => void;
}

const clamp = (value: string, min: number, max: number, fallback: number) => {
  const n = Math.round(Number(value));
  return Number.isFinite(n) && value.trim() !== "" ? Math.min(Math.max(n, min), max) : fallback;
};

/**
 * "File" button of the send box: a panel above it sets the chunk size and
 * the pause after each chunk, then asks for the file. Progress and Stop are
 * shown in the status bar.
 */
export function FileSend({ t, enabled, onStart }: Props) {
  const [chunk, setChunk] = useState(String(DEFAULT_FILE_OPTIONS.chunk));
  const [delay, setDelay] = useState(String(DEFAULT_FILE_OPTIONS.delayMs));
  return (
    <Popover
      label={t.sendFile}
      button={t.file}
      className="file-button"
      up
      alignRight
      disabled={!enabled}
    >
      {(close) => (
        <>
          <label title={t.chunkSizeHint}>
            {t.chunkSize}
            <input
              aria-label={t.chunkSize}
              type="number"
              min={1}
              max={MAX_CHUNK}
              className="gap-input"
              value={chunk}
              onChange={(e) => setChunk(e.target.value)}
            />
          </label>
          <label title={t.chunkDelayHint}>
            {t.chunkDelay}
            <input
              aria-label={t.chunkDelay}
              type="number"
              min={0}
              max={MAX_DELAY_MS}
              className="gap-input"
              value={delay}
              onChange={(e) => setDelay(e.target.value)}
            />
          </label>
          <button
            type="button"
            className="primary popover-wide"
            onClick={() => {
              const options = {
                chunk: clamp(chunk, 1, MAX_CHUNK, DEFAULT_FILE_OPTIONS.chunk),
                delayMs: clamp(delay, 0, MAX_DELAY_MS, DEFAULT_FILE_OPTIONS.delayMs),
              };
              setChunk(String(options.chunk));
              setDelay(String(options.delayMs));
              close();
              onStart(options);
            }}
          >
            {t.chooseFile}
          </button>
        </>
      )}
    </Popover>
  );
}
