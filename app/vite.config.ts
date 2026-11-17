import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// Tauri expects a fixed dev-server port (see src-tauri/tauri.conf.json).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  build: { target: "es2022" },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test-setup.ts"],
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/main.tsx", "src/test-setup.ts", "src/**/*.test.{ts,tsx}"],
      thresholds: { lines: 85 },
      reporter: ["text-summary", "text"],
    },
  },
});
