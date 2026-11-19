// `Chart` backed by uPlot. Glue only: it needs a canvas, so it is checked in
// the app rather than in jsdom (like xtermScreen.ts).
import uPlot from "uplot";
import "uplot/dist/uPlot.min.css";
import type { Chart, ChartTheme } from "./chart";

export function createUplotChart(element: HTMLElement, theme: ChartTheme): Chart {
  let plot: uPlot | null = null;
  let names: string[] = [];

  // uPlot puts its legend inside `element`, so the height comes from the
  // fixed-size container around it, less the legend; measuring `element`
  // itself would grow the chart on every resize.
  const size = () => {
    const box = element.parentElement ?? element;
    const legend = plot?.root.querySelector(".u-legend")?.clientHeight ?? 28;
    return {
      width: Math.max(box.clientWidth - 16, 100),
      height: Math.max(box.clientHeight - legend - 14, 60),
    };
  };

  const build = (series: string[], data: uPlot.AlignedData) => {
    plot?.destroy();
    names = series;
    const color = (i: number) => theme.series[i % theme.series.length] ?? theme.text;
    const grid = { stroke: theme.grid, width: 1 };
    // Every series has its own auto-ranged scale, so 23 °C and 1008 hPa both
    // fill the height; the first two get an axis (left, right) in their color.
    const valueAxes = series.slice(0, 2).map((_, i) => ({
      scale: `s${i}`,
      side: i === 0 ? 3 : 1,
      stroke: color(i),
      grid: { ...grid, show: i === 0 },
      ticks: { show: false },
    }));
    plot = new uPlot(
      {
        ...size(),
        legend: { show: true, live: true },
        cursor: { drag: { x: true, y: false } },
        scales: { x: { time: true } },
        axes: [{ stroke: theme.text, grid, ticks: { show: false } }, ...valueAxes],
        series: [
          {},
          ...series.map((label, i) => ({
            label,
            scale: `s${i}`,
            stroke: color(i),
            width: 1.5,
            spanGaps: true,
            points: { show: false },
          })),
        ],
      },
      data,
      element,
    );
    // The legend's height is known now: fit the plot area to it.
    plot.setSize(size());
  };

  return {
    setData: (times, series) => {
      const data = [times, ...series.map(([, values]) => values)] as uPlot.AlignedData;
      const wanted = series.map(([name]) => name);
      if (!plot || wanted.join("\u0000") !== names.join("\u0000")) build(wanted, data);
      else plot.setData(data);
    },
    resize: () => plot?.setSize(size()),
    dispose: () => {
      plot?.destroy();
      plot = null;
    },
  };
}
