import type { SignalsInfo } from "../api";
import type { Strings } from "../i18n";

interface Props {
  t: Strings;
  signals: SignalsInfo | null;
  /** Interactive only while the session is open. */
  enabled: boolean;
  onSet: (levels: { dtr?: boolean; rts?: boolean }) => void;
}

const level = (v: boolean | null) => (v === null ? "unknown" : v ? "on" : "off");

/**
 * DTR/RTS switches and CTS/DSR/RI/CD lamps. An output that was never set in
 * this session shows as unknown; clicking it asserts the line.
 */
export function SignalBar({ t, signals, enabled, onSet }: Props) {
  const out = (name: "dtr" | "rts") => {
    const value = signals?.[name] ?? null;
    return (
      <button
        type="button"
        className={`signal out ${level(value)}`}
        aria-pressed={value === true}
        aria-label={t.signalHint[name]}
        title={`${t.signalHint[name]} · ${value === null ? t.signalUnknown : value ? "1" : "0"}`}
        disabled={!enabled}
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
      {lamp("cts")}
      {lamp("dsr")}
      {lamp("ri")}
      {lamp("cd")}
    </span>
  );
}
