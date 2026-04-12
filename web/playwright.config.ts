import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  timeout: 30000,
  use: {
    headless: true,
  },
  webServer: {
    command: "npx vite --port 3123",
    port: 3123,
    reuseExistingServer: false,
  },
});
