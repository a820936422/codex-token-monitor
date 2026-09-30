import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/browser",
  timeout: 15_000,
  expect: { timeout: 4_000 },
  fullyParallel: true,
  workers: 2,
  reporter: "list",
  use: {
    browserName: "chromium",
    baseURL: "http://127.0.0.1:5178",
    viewport: { width: 1280, height: 760 },
    timezoneId: "UTC",
    screenshot: "only-on-failure",
    trace: "retain-on-failure",
  },
  webServer: {
    command: "npm run dev -- --port 5178",
    url: "http://127.0.0.1:5178/tests/browser/",
    reuseExistingServer: false,
    gracefulShutdown: { signal: "SIGTERM", timeout: 2_000 },
  },
});
