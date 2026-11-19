import { useEffect, useRef, useState } from "react";
import type { Chart, ChartFactory, ChartTheme } from "../chart";
import type { PlotData } from "../plot";

interface Props {
  data: PlotData;
  createChart: ChartFactory;
  theme: ChartTheme;
  /** Shown until the first number arrives. */
  emptyText: string;
  label: string;
  onError: (message: string) => void;
}

/**
 * Live chart of the numbers in received lines. Redraws at most once per
 * animation frame, and only when the data changed.
 */
export function PlotView({ data, createChart, theme, emptyText, label, onError }: Props) {
  const host = useRef<HTMLDivElement>(null);
  const [empty, setEmpty] = useState(data.times.length === 0);
  const fail = useRef(onError);
  fail.current = onError;

  useEffect(() => {
    const el = host.current;
    if (!el) return undefined;
    let chart: Chart | null = null;
    let gone = false;
    let frame = 0;
    let drawn = -1;
    const draw = () => {
      if (chart && drawn !== data.version) {
        drawn = data.version;
        chart.setData(data.times, [...data.series.entries()]);
        setEmpty(data.times.length === 0);
      }
      frame = requestAnimationFrame(draw);
    };
    const observer =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(() => chart?.resize());
    void Promise.resolve(createChart(el, theme)).then(
      (c) => {
        if (gone) return c.dispose();
        chart = c;
        observer?.observe(el);
        draw();
      },
      (e: unknown) => fail.current(String(e)),
    );
    return () => {
      gone = true;
      cancelAnimationFrame(frame);
      observer?.disconnect();
      chart?.dispose();
    };
  }, [data, createChart, theme]);

  return (
    <div className="plot" role="img" aria-label={label}>
      <div className="plot-canvas" ref={host} />
      {empty && <p className="plot-empty muted">{emptyText}</p>}
    </div>
  );
}
