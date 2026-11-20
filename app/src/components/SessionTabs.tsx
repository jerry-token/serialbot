import { type ReactNode, type WheelEvent, useEffect, useRef } from "react";
import type { SessionInfo } from "../api";
import type { Strings } from "../i18n";
import { Popover } from "./Popover";

/**
 * The short name a tab shows: the alias if there is one, otherwise the port
 * without the parts every port shares ("/dev/tty", "/dev/cu.", "\\.\").
 * Network addresses (tcp://, udp://, …) and anything unknown stay as they
 * are; a name that would become empty is kept whole.
 */
export function tabLabel(port: string, alias?: string | null): string {
  if (alias) return alias;
  let name = port;
  if (name.startsWith("\\\\.\\")) name = name.slice(4);
  if (name.startsWith("/dev/")) {
    name = name.slice(5);
    for (const prefix of ["tty.", "cu.", "tty"]) {
      if (name.startsWith(prefix) && name.length > prefix.length) {
        name = name.slice(prefix.length);
        break;
      }
    }
  }
  return name || port;
}

/** The full name, shown as the tab's tooltip. */
function fullName(s: SessionInfo): string {
  return s.alias ? `${s.port} (${s.alias})` : s.port;
}

interface Props {
  t: Strings;
  sessions: SessionInfo[];
  activeId: number | null;
  onSelect: (id: number) => void;
  onClose: (id: number) => void;
  /** View options, shown in their own row under the tabs. */
  children?: ReactNode;
}

/**
 * One tab per session, including sessions opened by MCP clients. Tabs show
 * short names and scroll sideways with the mouse wheel (no scroll bar over
 * them); with two or more sessions, ▾ lists them all by full name. The
 * view options get a row of their own, so the tabs have the full width.
 */
export function SessionTabs({
  t,
  sessions,
  activeId,
  onSelect,
  onClose,
  children,
}: Props) {
  const strip = useRef<HTMLDivElement>(null);

  // Keep the active tab in view when it changes or a tab is added.
  useEffect(() => {
    strip.current
      ?.querySelector(".tab.active")
      ?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [activeId, sessions.length]);

  // A vertical wheel scrolls the tabs sideways.
  const onWheel = (e: WheelEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    if (e.deltaX === 0 && e.deltaY !== 0) el.scrollLeft += e.deltaY;
  };

  return (
    <>
      <div className="tabs-row">
        <div className="tabs" role="tablist" ref={strip} onWheel={onWheel}>
          {sessions.map((s) => (
            <div
              key={s.id}
              className={`tab ${s.id === activeId ? "active" : ""}`}
            >
              <button
                type="button"
                role="tab"
                aria-selected={s.id === activeId}
                title={fullName(s)}
                onClick={() => onSelect(s.id)}
              >
                <span className={`dot ${s.state}`} aria-hidden="true" />
                <span className="tab-name">{tabLabel(s.port, s.alias)}</span>
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
        {sessions.length > 1 && (
          <Popover
            label={t.allSessions(sessions.length)}
            className="icon-button tabs-all"
            alignRight
            button="▾"
          >
            {(close) => (
              <ul className="popover-wide session-list">
                {sessions.map((s) => (
                  <li key={s.id}>
                    <button
                      type="button"
                      aria-current={s.id === activeId}
                      onClick={() => {
                        onSelect(s.id);
                        close();
                      }}
                    >
                      <span className={`dot ${s.state}`} aria-hidden="true" />
                      {fullName(s)}
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </Popover>
        )}
      </div>
      {children && <div className="view-options">{children}</div>}
    </>
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
