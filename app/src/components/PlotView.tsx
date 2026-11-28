// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { useEffect, useRef, useState } from "react";
import type { Chart, ChartFactory, ChartTheme } from "../chart";
import type { PlotData, PlotFormat } from "../plot";

interface Props {
  data: PlotData;
  createChart: ChartFactory;
  theme: ChartTheme;
  /** Shown until the first number arrives, per format. */
  emptyText: Record<PlotFormat, string>;
  label: string;
  /** The format picker's label and its choices' names (no picker without them). */
  formatLabel?: string;
  formatNames?: Record<PlotFormat, string>;
  /** The user picked another format (the data is read again in it). */
  onFormat?: (format: PlotFormat) => void;
  onError: (message: string) => void;
}

/**
 * Live chart of the numbers in received lines (or JustFloat frames), with a
 * picker for the format. Redraws at most once per animation frame, and only
 * when the data changed.
 */
export function PlotView({
  data,
  createChart,
  theme,
  emptyText,
  label,
  formatLabel,
  formatNames,
  onFormat,
  onError,
}: Props) {
  const host = useRef<HTMLDivElement>(null);
  const [format, setFormat] = useState<PlotFormat>(data.format);
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
    <div className="plot">
      <div className="plot-canvas" ref={host} role="img" aria-label={label} />
      {empty && <p className="plot-empty muted">{emptyText[format]}</p>}
      {formatNames && (
      <select
        className="plot-format"
        value={format}
        aria-label={formatLabel}
        title={formatLabel}
        onChange={(e) => {
          const next = e.target.value as PlotFormat;
          setFormat(next);
          onFormat?.(next);
        }}
      >
        {(Object.keys(formatNames) as PlotFormat[]).map((f) => (
          <option key={f} value={f}>
            {formatNames[f]}
          </option>
        ))}
      </select>
      )}
    </div>
  );
}
