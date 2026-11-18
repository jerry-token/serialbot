import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(() => {
  cleanup();
});


// jsdom has no layout. Give the log viewport and its rows sizes so the
// virtualized log renders rows (the virtualizer reads clientHeight of the
// viewport and offsetHeight of rows). A tall viewport shows every row of
// the small logs used in tests; virtualization tests override `viewport`.
export const layout = { viewport: 20_000, row: 18 };
const size = (el: HTMLElement, axis: "height" | "width") => {
  if (el.classList.contains("log")) return axis === "height" ? layout.viewport : 800;
  if (el.classList.contains("line")) return axis === "height" ? layout.row : 800;
  return 0;
};
for (const [prop, axis] of [
  ["clientHeight", "height"],
  ["offsetHeight", "height"],
  ["clientWidth", "width"],
  ["offsetWidth", "width"],
] as const) {
  Object.defineProperty(HTMLElement.prototype, prop, {
    configurable: true,
    get(this: HTMLElement) {
      return size(this, axis);
    },
  });
}

// jsdom does not scroll: keep scrollTop per element and fire "scroll" when
// scrollTo is called, as browsers do, so the virtualizer follows.
const scrollTops = new WeakMap<Element, number>();
Object.defineProperty(Element.prototype, "scrollTop", {
  configurable: true,
  get(this: Element) {
    return scrollTops.get(this) ?? 0;
  },
  set(this: Element, value: number) {
    scrollTops.set(this, value);
  },
});
Element.prototype.scrollTo = function (this: Element, arg?: ScrollToOptions | number, y?: number) {
  const top = typeof arg === "number" ? (y ?? 0) : (arg?.top ?? this.scrollTop);
  this.scrollTop = top;
  this.dispatchEvent(new Event("scroll"));
} as Element["scrollTo"];

// The scrollable height of the log is the height of its row container.
Object.defineProperty(HTMLElement.prototype, "scrollHeight", {
  configurable: true,
  get(this: HTMLElement) {
    const inner = this.firstElementChild as HTMLElement | null;
    return this.classList.contains("log") ? Number.parseFloat(inner?.style.height ?? "") || 0 : 0;
  },
});
