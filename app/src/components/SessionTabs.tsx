import type { SessionInfo } from "../api";
import type { Strings } from "../i18n";

interface Props {
  t: Strings;
  sessions: SessionInfo[];
  activeId: number | null;
  onSelect: (id: number) => void;
  onClose: (id: number) => void;
}

/** One tab per session, including sessions opened by MCP clients. */
export function SessionTabs({ t, sessions, activeId, onSelect, onClose }: Props) {
  if (sessions.length === 0) return null;
  return (
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
            onClick={() => onClose(s.id)}
          >
            ×
          </button>
        </div>
      ))}
    </div>
  );
}
