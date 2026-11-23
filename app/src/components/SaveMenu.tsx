// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { useState } from "react";
import type { CaptureStatus, ExportFormat } from "../api";
import { formatBytes } from "../api";
import type { Strings } from "../i18n";
import { SaveIcon } from "./icons";
import { Popover } from "./Popover";

interface Props {
  t: Strings;
  /** Disabled when no session is selected. */
  enabled: boolean;
  capture: CaptureStatus | null;
  /** Each returns a message to show, or null (e.g. the dialog was cancelled). */
  onExport: (format: ExportFormat) => Promise<string | null>;
  onStartCapture: (format: ExportFormat) => Promise<string | null>;
  onStopCapture: () => Promise<string | null>;
}

/** Saving the selected session's log: once, or continuously. */
export function SaveMenu({ t, enabled, capture, onExport, onStartCapture, onStopCapture }: Props) {
  const [format, setFormat] = useState<ExportFormat>("log");
  const [note, setNote] = useState<string | null>(null);
  const active = capture?.active === true;
  const run = (action: () => Promise<string | null>) => {
    setNote(null);
    action().then(setNote, (e: unknown) => setNote(String(e)));
  };

  return (
    <Popover
      label={t.save}
      className={`icon-button${active ? " recording" : ""}`}
      alignRight
      button={<SaveIcon size={16} />}
    >
      <label>
        {t.saveFormat}
        <select
          aria-label={t.saveFormat}
          value={format}
          disabled={active}
          onChange={(e) => setFormat(e.target.value as ExportFormat)}
        >
          <option value="log">{t.formatLog}</option>
          <option value="raw">{t.formatRaw}</option>
        </select>
      </label>
      <div className="popover-stack">
        <button type="button" disabled={!enabled} onClick={() => run(() => onExport(format))}>
          {t.exportNow}
        </button>
        {active ? (
          <button type="button" className="danger" onClick={() => run(onStopCapture)}>
            {t.stopCapture}
          </button>
        ) : (
          <button
            type="button"
            title={t.captureHint}
            disabled={!enabled}
            onClick={() => run(() => onStartCapture(format))}
          >
            {t.startCapture}
          </button>
        )}
        {active && capture?.path && (
          <span className="muted capture-path">
            {capture.path} · {formatBytes(capture.bytes)}
          </span>
        )}
        {note && <span className="note capture-path">{note}</span>}
      </div>
    </Popover>
  );
}
