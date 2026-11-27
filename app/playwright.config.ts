// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Jerry. SerialBot: https://github.com/jerry-token/serialbot
// Layout tests in a real browser (e2e/): the app with a fake backend, served
// by Vite. `npx playwright install chromium` once before `npm run e2e`.
import { defineConfig } from "@playwright/test";

// Node's, without pulling Node's types into the app's.
declare const process: { env: Record<string, string | undefined> };

const port = 5174;

export default defineConfig({
  testDir: "e2e",
  fullyParallel: true,
  reporter: process.env.CI ? "github" : "list",
  use: { baseURL: `http://localhost:${port}`, browserName: "chromium" },
  webServer: {
    command: `npx vite --port ${port} --strictPort`,
    url: `http://localhost:${port}/e2e/harness.html`,
    reuseExistingServer: !process.env.CI,
  },
});
