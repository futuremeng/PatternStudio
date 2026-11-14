import { defineConfig } from "@playwright/test";

/** Throwaway HOME seeded by tests/e2e/seed.cjs (run it before this suite).
 * Must match seed.cjs: os.tmpdir() varies between processes in sandboxed
 * environments, so pin a fixed path with the same PS_E2E_HOME override. */
export const PS_E2E_HOME = process.env.PS_E2E_HOME || "/tmp/patternstudio-e2e-home";

export default defineConfig({
  testDir: "./tests/e2e",
  // Specs share one server + seeded projects: run them sequentially, in file
  // order (empty -> errors -> workflow; empty recreates the project on exit).
  fullyParallel: false,
  workers: 1,
  timeout: 120_000,
  expect: { timeout: 60_000 },
  use: {
    baseURL: "http://127.0.0.1:8123",
    // The app follows navigator.language; force English so specs can assert
    // on the localized classification strings ("Invalid image: ...").
    locale: "en-US",
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium" }],
  webServer: {
    // Serves web/dist (build first) and the seeded project images. Debug
    // binary on purpose: its slower tracing widens the busy-window test.
    command: "target/debug/patternstudio-server",
    cwd: "..",
    env: { HOST: "127.0.0.1", PORT: "8123", HOME: PS_E2E_HOME },
    port: 8123,
    timeout: 180_000,
    reuseExistingServer: !process.env.CI,
  },
});
