// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
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
  /** Sessions shown in their own windows: no tab here, marked ⧉ in the list. */
  popped?: ReadonlySet<number>;
  /** Shows a session in its own window (double-click on a tab, or a list entry marked ⧉). */
  onPopOut?: (id: number) => void;
  /** Sessions with news (data, AI writes, a lost device) since they were last shown. */
  activity?: ReadonlySet<number>;
  /** Replaces the tab row (a session's own window shows its name instead). */
  header?: ReactNode;
  /** View options, shown in their own row under the tabs. */
  children?: ReactNode;
}

const NONE: ReadonlySet<number> = new Set();

/**
 * One tab per session, including sessions opened by MCP clients. Tabs show
 * short names and scroll sideways with the mouse wheel (no scroll bar over
 * them); ▾ lists every session by full name. A double-click moves a tab into
 * a window of its own; such sessions have no tab here and are marked ⧉ in
 * the list, where picking one brings its window to the front. The view
 * options get a row of their own, so the tabs have the full width.
 */
export function SessionTabs({
  t,
  sessions,
  activeId,
  onSelect,
  onClose,
  popped = NONE,
  onPopOut,
  activity = NONE,
  header,
  children,
}: Props) {
  const strip = useRef<HTMLDivElement>(null);
  const tabs = sessions.filter((s) => !popped.has(s.id));

  // Keep the active tab in view when it changes or a tab is added.
  useEffect(() => {
    strip.current
      ?.querySelector(".tab.active")
      ?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
  }, [activeId, tabs.length]);

  // A vertical wheel scrolls the tabs sideways.
  const onWheel = (e: WheelEvent<HTMLDivElement>) => {
    const el = e.currentTarget;
    if (e.deltaX === 0 && e.deltaY !== 0) el.scrollLeft += e.deltaY;
  };

  const row = header ? (
    <div className="tabs-row">{header}</div>
  ) : (
    <div className="tabs-row">
      <div className="tabs" role="tablist" ref={strip} onWheel={onWheel}>
        {tabs.map((s) => (
          <div key={s.id} className={`tab ${s.id === activeId ? "active" : ""}`}>
            <button
              type="button"
              role="tab"
              aria-selected={s.id === activeId}
              title={[
                fullName(s),
                activity.has(s.id) ? t.newActivity : null,
                onPopOut ? t.popOutByDoubleClick : null,
              ]
                .filter(Boolean)
                .join("\n")}
              onClick={() => onSelect(s.id)}
              onDoubleClick={() => onPopOut?.(s.id)}
            >
              <Dot session={s} news={activity.has(s.id)} />
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
      {(sessions.length > 1 || popped.size > 0) && (
        <Popover
          label={t.allSessions(sessions.length)}
          className="icon-button tabs-all"
          alignRight
          button="▾"
        >
          {(close) => (
            <ul className="popover-wide session-list">
              {sessions.map((s) => {
                const own = popped.has(s.id);
                return (
                  <li key={s.id}>
                    <button
                      type="button"
                      aria-current={s.id === activeId}
                      title={own ? t.inOwnWindow : undefined}
                      onClick={() => {
                        if (own) onPopOut?.(s.id);
                        else onSelect(s.id);
                        close();
                      }}
                    >
                      <Dot session={s} news={activity.has(s.id)} />
                      {fullName(s)}
                      {own && <span className="own-window"> ⧉</span>}
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </Popover>
      )}
    </div>
  );

  return (
    <>
      {row}
      {children && <div className="view-options">{children}</div>}
    </>
  );
}

/** A session's state dot; with news it gets a ring in the accent colour (the tooltip says why). */
function Dot({ session, news }: { session: SessionInfo; news: boolean }) {
  return <span className={`dot ${session.state}${news ? " news" : ""}`} aria-hidden="true" />;
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
