// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import type { SequenceStatus } from "../api";
import type { CommandGroup } from "../commands";
import type { Strings } from "../i18n";
import {
  MAX_GAP_MS,
  type SequenceSettings,
  averageMs,
  failureText,
  groupToScript,
} from "../sequence";

/** How many failures the panel lists; the report has all the backend kept. */
const SHOWN_FAILURES = 5;

interface Props {
  t: Strings;
  settings: SequenceSettings;
  onChange: (settings: SequenceSettings) => void;
  /** Groups the script can be filled from. */
  groups: CommandGroup[];
  /** Whether a sequence can start (the selected session is open). */
  enabled: boolean;
  /** The selected session's sequence, if it has had one. */
  status: SequenceStatus | null;
  onRun: () => void;
  onStop: () => void;
  onCopyReport: () => void;
}

function whole(text: string, max: number): number {
  const n = Number.parseInt(text, 10);
  return Number.isFinite(n) ? Math.min(Math.max(n, 0), max) : 0;
}

/**
 * The test-sequence panel beside the log: a script, how often to run it,
 * Run/Stop, and the tally with the first failures. The sequence runs in
 * the backend, so it goes on while the panel is closed.
 */
export function SequencePanel({
  t,
  settings,
  onChange,
  groups,
  enabled,
  status,
  onRun,
  onStop,
  onCopyReport,
}: Props) {
  const running = status?.running === true;
  const set = (change: Partial<SequenceSettings>) =>
    onChange({ ...settings, ...change });
  const average = status ? averageMs(status) : null;
  return (
    <aside className="command-panel sequence-panel" aria-label={t.sequence}>
      <div className="command-head">
        <strong title={t.testSequenceHint}>{t.sequence}</strong>
        <select
          aria-label={t.sequenceFromGroup}
          value=""
          disabled={running}
          onChange={(e) => {
            const group = groups.find((g) => g.id === e.target.value);
            if (group) set({ script: groupToScript(group) });
          }}
        >
          <option value="">{t.sequenceFromGroup}</option>
          {groups.map((g) => (
            <option key={g.id} value={g.id}>
              {g.name}
            </option>
          ))}
        </select>
      </div>
      <div className="sequence-body">
        <textarea
          aria-label={t.sequenceScript}
          spellCheck={false}
          readOnly={running}
          value={settings.script}
          onChange={(e) => set({ script: e.target.value })}
        />
        <p className="muted small">{t.sequenceSyntax}</p>
        <label className="sequence-field">
          <span>{t.sequenceRuns}</span>
          <input
            type="number"
            min={0}
            disabled={running}
            value={settings.runs}
            onChange={(e) =>
              set({ runs: whole(e.target.value, 1_000_000_000) })
            }
          />
        </label>
        <label className="sequence-field">
          <span>{t.sequenceGap}</span>
          <input
            type="number"
            min={0}
            max={MAX_GAP_MS}
            disabled={running}
            value={settings.gapMs}
            onChange={(e) => set({ gapMs: whole(e.target.value, MAX_GAP_MS) })}
          />
        </label>
        <label className="check">
          <input
            type="checkbox"
            disabled={running}
            checked={settings.keepGoing}
            onChange={(e) => set({ keepGoing: e.target.checked })}
          />
          {t.sequenceKeepGoing}
        </label>
        {status && (status.running || status.done > 0) && (
          <div className="sequence-status" aria-live="polite">
            {status.running && (
              <p>
                {t.sequenceProgress(status.done + 1, status.runs)}
                {status.step && (
                  <code className="sequence-step"> {status.step}</code>
                )}
              </p>
            )}
            <p className={status.failed > 0 ? "error" : "ok"}>
              {t.sequenceTally(status.done, status.passed, status.failed)}
              {status.stopped && ` (${t.sequenceStopped})`}
            </p>
            {average !== null &&
              status.fastestMs !== null &&
              status.slowestMs !== null && (
                <p className="muted">
                  {t.sequenceTimes(status.fastestMs, average, status.slowestMs)}
                </p>
              )}
            {status.error && <p className="error">{status.error}</p>}
            {status.failures.length > 0 && (
              <ul className="sequence-failures">
                {status.failures.slice(0, SHOWN_FAILURES).map((f) => (
                  <li key={f.run}>{failureText(t, f)}</li>
                ))}
                {status.failed > SHOWN_FAILURES && (
                  <li className="muted">
                    {t.sequenceMoreFailures(status.failed - SHOWN_FAILURES)}
                  </li>
                )}
              </ul>
            )}
          </div>
        )}
      </div>
      <div className="command-foot">
        {running ? (
          <button type="button" className="danger" onClick={onStop}>
            {t.sequenceStop}
          </button>
        ) : (
          <button
            type="button"
            className="primary"
            disabled={!enabled || settings.script.trim() === ""}
            onClick={onRun}
          >
            {t.sequenceRun}
          </button>
        )}
        {status && status.done > 0 && (
          <button type="button" className="chip" onClick={onCopyReport}>
            {t.copyReport}
          </button>
        )}
      </div>
    </aside>
  );
}
