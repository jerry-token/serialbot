import { useLayoutEffect, useRef } from "react";
import type { Line } from "../terminal";
import { formatTime } from "../timefmt";

interface Props {
  lines: readonly Line[];
  showTimestamps: boolean;
  showSent: boolean;
  autoScroll: boolean;
  /** Changes whenever the lines change, so the view scrolls. */
  version: number;
}

const PREFIX: Record<Line["kind"], string> = { rx: "", tx: "→ ", marker: "# ", info: "! " };

export function LogView({ lines, showTimestamps, showSent, autoScroll, version }: Props) {
  const ref = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const el = ref.current;
    if (autoScroll && el) el.scrollTop = el.scrollHeight;
  }, [version, autoScroll]);

  return (
    <div className="log" ref={ref} role="log" aria-live="off">
      {lines
        .filter((line) => showSent || line.kind !== "tx")
        .map((line) => (
          <div key={line.id} className={`line ${line.kind}`}>
            {showTimestamps && <span className="ts">[{formatTime(line.timestampUs)}] </span>}
            {PREFIX[line.kind]}
            {line.text}
          </div>
        ))}
    </div>
  );
}
