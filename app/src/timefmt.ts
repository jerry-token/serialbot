// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
/** Local wall-clock time with milliseconds, e.g. "14:03:27.481". */
export function formatTime(timestampUs: number): string {
  const date = new Date(Math.floor(timestampUs / 1000));
  const pad = (n: number, width = 2) => String(n).padStart(width, "0");
  return (
    `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}` +
    `.${pad(date.getMilliseconds(), 3)}`
  );
}
