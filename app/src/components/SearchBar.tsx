// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { useEffect, useRef } from "react";
import type { Strings } from "../i18n";
import { ChevronIcon, UpIcon } from "./icons";

interface Props {
  t: Strings;
  query: string;
  onQuery: (query: string) => void;
  /** 1-based position of the current match; 0 when there is none. */
  current: number;
  total: number;
  onStep: (delta: 1 | -1) => void;
  onClose: () => void;
  /** Whether the log shows only matching lines. */
  filter: boolean;
  onFilter: (on: boolean) => void;
}

/**
 * Find bar over the log: Enter / Shift+Enter step through matches, Escape
 * closes. With the filter on, the log shows only the matching lines.
 */
export function SearchBar({
  t,
  query,
  onQuery,
  current,
  total,
  onStep,
  onClose,
  filter,
  onFilter,
}: Props) {
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);
  return (
    <div className="searchbar" role="search">
      <input
        ref={input}
        type="search"
        aria-label={t.search}
        placeholder={t.searchPlaceholder}
        value={query}
        spellCheck={false}
        onChange={(e) => onQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            onStep(e.shiftKey ? -1 : 1);
          } else if (e.key === "Escape") {
            e.preventDefault();
            onClose();
          }
        }}
      />
      <label className="check filter" title={t.filterMatches}>
        <input type="checkbox" checked={filter} onChange={(e) => onFilter(e.target.checked)} />
        {t.filterMatches}
      </label>
      <span className="count" aria-live="polite">
        {query ? t.searchCount(current, total) : ""}
      </span>
      <button
        type="button"
        className="icon-button"
        aria-label={t.previousMatch}
        title={t.previousMatch}
        disabled={total === 0}
        onClick={() => onStep(-1)}
      >
        <UpIcon size={14} />
      </button>
      <button
        type="button"
        className="icon-button"
        aria-label={t.nextMatch}
        title={t.nextMatch}
        disabled={total === 0}
        onClick={() => onStep(1)}
      >
        <ChevronIcon size={14} />
      </button>
      <button
        type="button"
        className="icon-button"
        aria-label={t.closeSearch}
        title={t.closeSearch}
        onClick={onClose}
      >
        ×
      </button>
    </div>
  );
}
