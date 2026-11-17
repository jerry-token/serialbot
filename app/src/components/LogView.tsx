import { useLayoutEffect, useRef } from "react";
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
            {line.text}
          </div>
        );
      })}
    </div>
  );
}
