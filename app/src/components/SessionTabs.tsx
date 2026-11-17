import type { ReactNode } from "react";
import type { SessionInfo } from "../api";
import type { Strings } from "../i18n";

interface Props {
  t: Strings;
  sessions: SessionInfo[];
  activeId: number | null;
  onSelect: (id: number) => void;
  onClose: (id: number) => void;
  /** Controls shown at the right end of the row (view options). */
  children?: ReactNode;
}

/** One tab per session, including sessions opened by MCP clients. */
export function SessionTabs({ t, sessions, activeId, onSelect, onClose, children }: Props) {
  return (
    <div className="tabs-row">
      <div className="tabs" role="tablist">
        {sessions.map((s) => (
          <div key={s.id} className={`tab ${s.id === activeId ? "active" : ""}`}>
            <button
              type="button"
              role="tab"
              aria-selected={s.id === activeId}
              onClick={() => onSelect(s.id)}
            >
              <span className={`dot ${s.state}`} aria-hidden="true" />
              {s.port}
              {s.alias && <span className="alias"> ({s.alias})</span>}
            </button>
            <button
              type="button"
              className="tab-close"
              aria-label={`${t.closeSession} ${s.port}`}
              title={t.closeSession}
              onClick={() => onClose(s.id)}
            >
              ×
            </button>
          </div>
        ))}
      </div>
      <div className="view-options">{children}</div>
    </div>
  );
}

interface ToggleProps {
  label: string;
  hint: string;
  pressed: boolean;
  onChange: (pressed: boolean) => void;
}

/** A small on/off chip, e.g. for "show timestamps". */
export function Toggle({ label, hint, pressed, onChange }: ToggleProps) {
  return (
    <button
      type="button"
      className="chip"
      aria-pressed={pressed}
      aria-label={hint}
      title={hint}
      onClick={() => onChange(!pressed)}
    >
      {label}
    </button>
  );
}
