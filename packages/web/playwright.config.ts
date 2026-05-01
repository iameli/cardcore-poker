import { existsSync } from "node:fs";
import { defineConfig } from "@playwright/test";

// Resolve a Chromium binary. In Docker / supported distros Playwright installs
// its own bundled chromium and we use that. On Ubuntu 26.04 (where Playwright
// doesn't yet ship a binary) we point at the system snap. Override with the
// PLAYWRIGHT_CHROMIUM env var.
function resolveChromium(): string | undefined {
  if (process.env.PLAYWRIGHT_CHROMIUM) return process.env.PLAYWRIGHT_CHROMIUM;
  for (const path of ["/usr/bin/chromium-browser", "/usr/bin/chromium"]) {
    if (existsSync(path)) return path;
  }
  return undefined; // fall back to Playwright's bundled chromium
}

const SYSTEM_CHROMIUM = resolveChromium();

export default defineConfig({
  testDir: "./tests",
  testMatch: /.*\.spec\.ts$/,
  timeout: 60_000,
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL: "http://127.0.0.1:5173",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    headless: SYSTEM_CHROMIUM ? false : true,
    viewport: { width: 1280, height: 800 },
    launchOptions: SYSTEM_CHROMIUM
      ? {
          executablePath: SYSTEM_CHROMIUM,
          args: ["--headless=new", "--no-sandbox", "--disable-gpu"],
        }
      : {
          args: ["--no-sandbox"],
        },
  },
  projects: [{ name: "chromium" }],
  webServer: {
    command: "pnpm run dev:all",
    url: "http://127.0.0.1:5173",
    reuseExistingServer: false,
    timeout: 30_000,
    stdout: "pipe",
    stderr: "pipe",
  },
});
