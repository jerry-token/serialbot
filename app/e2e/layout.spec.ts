// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
// Nothing may stick out of the window, sideways or up and down: from the
// smallest window the app allows to a large one, in English and Chinese,
// with a port open, HEX on, the longest choice picked in every dropdown and
// each panel opened. A real browser lays the page out; jsdom cannot.
import { expect, type Page, test } from "@playwright/test";

/**
 * Elements reaching past the window, unless a box inside the window that
 * scrolls or clips holds them.
 */
async function overflow(page: Page): Promise<string[]> {
  return page.evaluate(() => {
    const width = window.innerWidth;
    const height = window.innerHeight;
    const found: string[] = [];
    const root = document.getElementById("root");
    // Whether a box between el and #root scrolls or clips along this axis
    // and itself lies inside the window (#root's own clipping does not
    // count: that is the window's edge).
    const scrolls = (el: Element, axis: "overflowX" | "overflowY") => {
      for (let p = el.parentElement; p && p !== root; p = p.parentElement) {
        if (getComputedStyle(p)[axis] === "visible") continue;
        const b = p.getBoundingClientRect();
        const inside =
          axis === "overflowX"
            ? b.right <= width + 0.5
            : b.bottom <= height + 0.5;
        if (inside) return true;
      }
      return false;
    };
    for (const el of document.querySelectorAll("#root *")) {
      const r = el.getBoundingClientRect();
      if (r.width === 0 || r.height === 0) continue;
      // Wholly left of or above the window: placed off screen on purpose
      // (text measuring, screen-reader-only text), not pushed out.
      if (r.right <= 0 || r.bottom <= 0) continue;
      const wide = r.right > width + 0.5 || r.left < -0.5;
      const tall = r.bottom > height + 0.5 || r.top < -0.5;
      const past =
        (wide && !scrolls(el, "overflowX")) ||
        (tall && !scrolls(el, "overflowY"));
      if (!past) continue;
      const label =
        el.getAttribute("aria-label") ??
        el.textContent?.trim().slice(0, 30) ??
        "";
      const box = [r.left, r.top, r.right, r.bottom].map(Math.round).join(", ");
      found.push(
        `<${el.tagName.toLowerCase()} class="${el.getAttribute("class") ?? ""}"> "${label}" at ${box}`,
      );
    }
    const doc = document.documentElement;
    if (doc.scrollWidth > width || doc.scrollHeight > height) {
      found.push(`the page is ${doc.scrollWidth} × ${doc.scrollHeight} px`);
    }
    return found;
  });
}

async function fits(page: Page, step: string) {
  expect(await overflow(page), `${step}: elements past the window`).toEqual([]);
}

/** Picks the option with the longest text in every visible dropdown. */
async function longestChoices(page: Page) {
  for (const select of await page.locator("select:visible:enabled").all()) {
    const value = await select.evaluate((s: HTMLSelectElement) => {
      const options = [...s.options].filter((o) => !o.disabled);
      return options.reduce(
        (a, b) => (b.text.length > a.text.length ? b : a),
        options[0]!,
      ).value;
    });
    await select.selectOption(value);
  }
}

// The smallest window the app allows (640 × 420, tauri.conf.json), the
// default (1000 × 720) and a large one.
const SIZES = [
  { width: 640, height: 420 },
  { width: 1000, height: 720 },
  { width: 1440, height: 900 },
];

/** Buttons that end something rather than open it. */
const SKIP = /close|clear|send|stop|关闭|清空|发送|停止/i;

/**
 * Clicks every enabled button of the current page in turn (looked up by
 * name, as clicks change the page), checking the layout with each opened.
 */
async function openEachPanel(page: Page, where: string) {
  const names = await page
    .locator("#root button:visible:enabled:not(.nav-item)")
    .evaluateAll((buttons) =>
      buttons.map((b) =>
        (b.getAttribute("aria-label") ?? b.textContent ?? "").trim(),
      ),
    );
  for (const name of new Set(names)) {
    if (!name || SKIP.test(name)) continue;
    const button = page.getByRole("button", { name, exact: true }).first();
    if (!(await button.isVisible()) || !(await button.isEnabled())) continue;
    await button.click({ timeout: 5000 });
    await longestChoices(page);
    await fits(page, `${where}: after "${name}"`);
    await page.keyboard.press("Escape");
  }
}

for (const locale of ["en-US", "zh-CN"]) {
  for (const viewport of SIZES) {
    test.describe(`${locale} at ${viewport.width} × ${viewport.height}`, () => {
      test.use({ locale, viewport });

      test("nothing sticks out of the window", async ({ page }) => {
        const errors: string[] = [];
        page.on("pageerror", (e) => errors.push(e.message));
        await page.goto("/e2e/harness.html");
        await expect(page.locator(".send-controls")).toBeVisible();
        await fits(page, "start");

        await page.locator(".toolbar button.primary").first().click();
        await expect(page.locator(".statusbar .state.open")).toBeVisible();
        await fits(page, "port open");

        await page
          .locator(".send-controls input[type=checkbox]")
          .first()
          .check();
        await fits(page, "HEX send");
        await longestChoices(page);
        await fits(page, "longest choices");

        // Each page of the sidebar, and on it every button that opens something.
        const pages = await page.locator(".nav-item").count();
        for (let i = 0; i < pages; i++) {
          const nav = page.locator(".nav-item").nth(i);
          const pageName =
            (await nav.getAttribute("aria-label")) ??
            (await nav.textContent()) ??
            "";
          await nav.click();
          await fits(page, `page "${pageName.trim()}"`);
          await openEachPanel(page, pageName.trim());
        }
        expect(errors).toEqual([]);
      });

      test("the frames window fits", async ({ page }) => {
        const errors: string[] = [];
        page.on("pageerror", (e) => errors.push(e.message));
        await page.goto("/e2e/harness.html#frames");
        await expect(page.locator(".frames-table .frame").first()).toBeVisible();
        await fits(page, "frames");
        await page.locator(".frames-table .frame").first().click();
        await expect(page.locator(".frame-detail")).toBeVisible();
        await fits(page, "frame fields");
        expect(errors).toEqual([]);
      });
    });
  }
}
