// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { EDGE, Popover, shiftIntoWindow } from "./Popover";

describe("shiftIntoWindow", () => {
  it("leaves a panel that fits where it is", () => {
    expect(shiftIntoWindow(EDGE, 600, 640)).toBe(0);
  });

  it("moves a panel past the right edge back in", () => {
    expect(shiftIntoWindow(400, 800, 640)).toBe(640 - EDGE - 800);
  });

  it("moves a panel past the left edge back in", () => {
    expect(shiftIntoWindow(-30, 200, 640)).toBe(EDGE + 30);
  });

  it("puts a panel wider than the window at the left margin", () => {
    expect(shiftIntoWindow(300, 1100, 640)).toBe(EDGE - 300);
  });
});

describe("Popover", () => {
  afterEach(() => vi.restoreAllMocks());

  it("shifts its panel into the window when it opens", () => {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
      left: 500,
      right: 760,
      top: 0,
      bottom: 100,
      width: 260,
      height: 100,
      x: 500,
      y: 0,
      toJSON: () => ({}),
    });
    Object.defineProperty(window, "innerWidth", { value: 640, configurable: true });
    render(
      <Popover label="Panel" button="Open">
        <p>content</p>
      </Popover>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Panel" }));
    const panel = screen.getByRole("dialog", { name: "Panel" });
    expect(panel.style.translate).toBe(`${640 - EDGE - 760}px`);

    // The window grows: it fits again.
    Object.defineProperty(window, "innerWidth", { value: 1200, configurable: true });
    fireEvent(window, new Event("resize"));
    expect(panel.style.translate).toBe("");
  });
});
