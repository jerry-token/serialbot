// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
// The chart behind the Plot view, behind a small interface so the UI can be
// tested without a real canvas.

/** Colors for the chart, from the app's theme. */
export interface ChartTheme {
  text: string;
  grid: string;
  /** One color per series, used in turn. */
  series: string[];
}

/** A live line chart drawn into an element. */
export interface Chart {
  /** Replaces the data: shared times (seconds) and named series aligned with them. */
  setData(times: number[], series: Array<[string, Array<number | null>]>): void;
  /** Fits the chart to its element. */
  resize(): void;
  dispose(): void;
}

export type ChartFactory = (element: HTMLElement, theme: ChartTheme) => Chart | Promise<Chart>;

/** The real chart (uPlot), loaded on first use. */
export const loadUplotChart: ChartFactory = async (element, theme) =>
  (await import("./uplotChart")).createUplotChart(element, theme);

/** Chart colors from the CSS variables in effect (ANSI palette for the series). */
export function chartThemeFromCss(style: CSSStyleDeclaration): ChartTheme {
  const v = (name: string, fallback: string) => style.getPropertyValue(name).trim() || fallback;
  const series = [4, 1, 2, 5, 6, 3, 12, 9, 10, 13, 14, 11].map((i) =>
    v(`--ansi-${i}`, ["#0451a5", "#cd3131", "#107c10", "#bc05bc"][i % 4] ?? "#0451a5"),
  );
  return { text: v("--muted", "#6b7480"), grid: v("--border", "#dcdfe4"), series };
}
