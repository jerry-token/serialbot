import { useVirtualizer } from "@tanstack/react-virtual";
import { type CSSProperties, type ReactNode, useLayoutEffect, useRef } from "react";
import type { Style } from "../ansi";
import type { Highlighter } from "../highlight";
import type { Line } from "../terminal";
import { formatTime } from "../timefmt";

interface Props {
  /** Lines to show (already filtered). */
  lines: readonly Line[];
  showTimestamps: boolean;
  /** Show the time since the previous shown line. */
  showDelta?: boolean;
  autoScroll: boolean;
  /** Changes whenever the lines change, so the view scrolls. */
  version: number;
  /** Ids of lines that match the search, if searching. */
  matches?: ReadonlySet<number>;
  /** The match to scroll to. */
  currentMatch?: number | null;
  /** Marks the user's keywords, if any. */
  highlight?: Highlighter | null;
}

/** `text` with the keywords marked, or as is without a highlighter. */
function marked(text: string, highlight: Highlighter | null): ReactNode {
  if (!highlight) return text;
  // Segments never reorder within a text, so the index is a stable key.
  return highlight(text).map((s, i) =>
    s.color ? (
      <mark key={i} className="kw" style={{ backgroundColor: s.color }}>
        {s.text}
      </mark>
    ) : (
      s.text
    ),
  );
}

const PREFIX: Record<Line["kind"], string> = { rx: "", tx: "→ ", marker: "# ", info: "! " };

/** CSS for an ANSI style; inverse swaps the colors (defaults included). */
export function styleCss(style: Style): CSSProperties {
  const css: CSSProperties = {};
  const fg = style.inverse ? (style.bg ?? "var(--log-bg)") : style.fg;
  const bg = style.inverse ? (style.fg ?? "var(--rx)") : style.bg;
  if (fg) css.color = fg;
  if (bg) css.backgroundColor = bg;
  if (style.bold) css.fontWeight = 600;
  if (style.dim) css.opacity = 0.7;
  if (style.italic) css.fontStyle = "italic";
  if (style.underline) css.textDecoration = "underline";
  return css;
}

/**
 * Time between two lines: "+0.8ms" under 10 ms, "+12ms" under a second,
 * "+1.234s" under a minute, "+2m03s" beyond. Lines out of order (a clock
 * step) show "+0ms" rather than a negative gap.
 */
export function formatDelta(us: number): string {
  const d = Math.max(0, us);
  if (d < 10_000) return `+${(d / 1000).toFixed(1)}ms`;
  if (d < 1_000_000) return `+${Math.round(d / 1000)}ms`;
  if (d < 60_000_000) return `+${(d / 1_000_000).toFixed(3)}s`;
  const s = Math.floor(d / 1_000_000);
  return `+${Math.floor(s / 60)}m${String(s % 60).padStart(2, "0")}s`;
}

/** Estimated height of one unwrapped row, refined by measuring rendered rows. */
const ROW_HEIGHT = 18;

/**
 * The log, virtualized: only rows near the viewport are in the DOM, so tens
 * of thousands of lines stay smooth. Rows wrap, so their heights are
 * measured as they render.
 */
export function LogView({
  lines,
  showTimestamps,
  showDelta = false,
  autoScroll,
  version,
  matches,
  currentMatch = null,
  highlight = null,
}: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: lines.length,
    getScrollElement: () => ref.current,
    estimateSize: () => ROW_HEIGHT,
    getItemKey: (index) => lines[index]?.id ?? index,
    overscan: 20,
    // Used until the element is measured (and in tests without layout).
    initialRect: { width: 800, height: 600 },
  });

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (currentMatch !== null) {
      const index = lines.findIndex((line) => line.id === currentMatch);
      if (index >= 0) virtualizer.scrollToIndex(index, { align: "center" });
    } else if (autoScroll && lines.length > 0) {
      virtualizer.scrollToIndex(lines.length - 1, { align: "end" });
    }
    // `lines` changes with `version`; the virtualizer object is stable.
  }, [version, autoScroll, currentMatch, lines.length]);

  return (
    <div className="log" ref={ref} role="log" aria-live="off">
      <div className="log-rows" style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map((item) => {
          const line = lines[item.index];
          if (!line) return null;
          const match = matches?.has(line.id) ? " match" : "";
          const current = line.id === currentMatch ? " current" : "";
          const ai = line.ai ? " ai" : "";
          return (
            <div
              key={item.key}
              data-index={item.index}
              ref={virtualizer.measureElement}
              className={`line ${line.kind}${ai}${match}${current}`}
              style={{ transform: `translateY(${item.start}px)` }}
            >
              {showTimestamps && <span className="ts">[{formatTime(line.timestampUs)}] </span>}
              {showDelta && (
                <span className="delta">
                  {item.index > 0
                    ? formatDelta(line.timestampUs - (lines[item.index - 1]?.timestampUs ?? 0))
                    : ""}{" "}
                </span>
              )}
              {line.ai && <span className="ai-tag">AI</span>}
              {PREFIX[line.kind]}
              {line.parts
                ? line.parts.map((part, i) =>
                    part.style ? (
                      // Parts never reorder within a line, so the index is a stable key.
                      <span key={i} style={styleCss(part.style)}>
                        {marked(part.text, highlight)}
                      </span>
                    ) : (
                      <span key={i}>{marked(part.text, highlight)}</span>
                    ),
                  )
                : marked(line.text, highlight)}
            </div>
          );
        })}
      </div>
    </div>
  );
}
