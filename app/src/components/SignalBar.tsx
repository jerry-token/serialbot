import { useState } from "react";
import type { SignalsInfo } from "../api";
import type { Strings } from "../i18n";
import { parseSequence, type SignalStep } from "../signals";
import { Popover } from "./Popover";

interface Props {
  t: Strings;
  signals: SignalsInfo | null;
  /** Interactive only while the session is open. */
  enabled: boolean;
  onSet: (levels: { dtr?: boolean; rts?: boolean }) => void;
  /** Sends a BREAK of `BREAK_MS`. */
  onBreak: () => void;
  /** The DTR/RTS sequence last typed, kept by the caller. */
  sequence: string;
  onSequenceChange: (text: string) => void;
  onRunSequence: (steps: SignalStep[]) => void;
}

/** Length of the BREAK button's break. */
export const BREAK_MS = 250;

const level = (v: boolean | null) => (v === null ? "unknown" : v ? "on" : "off");

/**
 * DTR/RTS switches and CTS/DSR/RI/CD lamps. An output that was never set in
 * this session shows as unknown; clicking it asserts the line. RTS is
 * disabled, with an explanation, while hardware flow control drives it.
 * BREAK holds a break for `BREAK_MS`; the sequence popover runs a typed
 * DTR/RTS sequence (see `parseSequence`) with backend-timed pauses.
 */
export function SignalBar({
  t,
  signals,
  enabled,
  onSet,
  onBreak,
  sequence,
  onSequenceChange,
  onRunSequence,
}: Props) {
  const [problem, setProblem] = useState<string | null>(null);
  const out = (name: "dtr" | "rts") => {
    const value = signals?.[name] ?? null;
    const locked = name === "rts" && signals?.rtsManual === false;
    const state = value === null ? t.signalUnknown : value ? "1" : "0";
    return (
      <button
        type="button"
        className={`signal out ${level(value)}${locked ? " locked" : ""}`}
        aria-pressed={value === true}
        aria-label={t.signalHint[name]}
        title={locked ? t.rtsLocked : `${t.signalHint[name]} · ${state}`}
        disabled={!enabled || locked}
        onClick={() => onSet({ [name]: value !== true })}
      >
        {name.toUpperCase()}
      </button>
    );
  };
  const lamp = (name: "cts" | "dsr" | "ri" | "cd") => {
    const value = signals?.[name] ?? null;
    return (
      <span
        className={`signal in ${level(value)}`}
        role="img"
        aria-label={`${t.signalHint[name]}: ${value === null ? t.signalUnknown : value ? "1" : "0"}`}
        title={t.signalHint[name]}
      >
        {name.toUpperCase()}
      </span>
    );
  };
  return (
    <span className="signals">
      {out("dtr")}
      {out("rts")}
      <button
        type="button"
        className="signal out"
        title={t.breakHint(BREAK_MS)}
        disabled={!enabled}
        onClick={onBreak}
      >
        BRK
      </button>
      <Popover
        label={t.signalSequence}
        button="⏱"
        className="signal out sequence"
        up
        disabled={!enabled}
      >
        {(close) => (
          <form
            className="sequence-form"
            onSubmit={(e) => {
              e.preventDefault();
              try {
                const steps = parseSequence(sequence);
                setProblem(null);
                onRunSequence(steps);
                close();
              } catch (err) {
                setProblem(String(err));
              }
            }}
          >
            <label>
              {t.signalSequence}
              <textarea
                aria-label={t.signalSequence}
                rows={3}
                value={sequence}
                placeholder="DTR=0 RTS=1, 100, RTS=0"
                onChange={(e) => onSequenceChange(e.target.value)}
              />
            </label>
            <p className="muted">{t.sequenceHint}</p>
            {problem && <p className="error">{problem}</p>}
            <button type="submit" className="primary">
              {t.runSequence}
            </button>
          </form>
        )}
      </Popover>
      {lamp("cts")}
      {lamp("dsr")}
      {lamp("ri")}
      {lamp("cd")}
    </span>
  );
}
