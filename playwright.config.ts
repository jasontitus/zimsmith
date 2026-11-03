import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./tests",
  timeout: 90_000,
  use: { baseURL: "http://127.0.0.1:1420", trace: "retain-on-failure" },
  workers: 1,
});
