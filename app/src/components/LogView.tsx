import { type CSSProperties, useLayoutEffect, useRef } from "react";
import type { Style } from "../ansi";
import type { Line } from "../terminal";
import { formatTime } from "../timefmt";

interface Props {
  /** Lines to show (already filtered). */
  lines: readonly Line[];
  showTimestamps: boolean;
  autoScroll: boolean;
  /** Changes whenever the lines change, so the view scrolls. */
  version: number;
  /** Ids of lines that match the search, if searching. */
  matches?: ReadonlySet<number>;
  /** The match to scroll to. */
  currentMatch?: number | null;
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

export function LogView({
  lines,
  showTimestamps,
  autoScroll,
  version,
  matches,
  currentMatch = null,
}: Props) {
  const ref = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (currentMatch !== null) {
      el.querySelector(".line.current")?.scrollIntoView?.({ block: "center" });
    } else if (autoScroll) {
      el.scrollTop = el.scrollHeight;
    }
  }, [version, autoScroll, currentMatch]);

  return (
    <div className="log" ref={ref} role="log" aria-live="off">
      {lines.map((line) => {
        const match = matches?.has(line.id) ? " match" : "";
        const current = line.id === currentMatch ? " current" : "";
        return (
          <div key={line.id} className={`line ${line.kind}${match}${current}`}>
            {showTimestamps && <span className="ts">[{formatTime(line.timestampUs)}] </span>}
            {PREFIX[line.kind]}
            {line.parts
              ? line.parts.map((part, i) =>
                  part.style ? (
                    // Parts never reorder within a line, so the index is a stable key.
                    <span key={i} style={styleCss(part.style)}>
                      {part.text}
                    </span>
                  ) : (
                    part.text
                  ),
                )
              : line.text}
          </div>
        );
      })}
    </div>
  );
}
