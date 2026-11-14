// End-to-end tests: the e2e build of the extension in Chromium against a stub
// native host (e2e/). Build first: `pnpm --filter @boltget/extension e2e`.
// BOLTGET_E2E_BROWSER runs another Chromium build (an installed Edge, say).
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "e2e",
  // Each test registers the same stub host name: one at a time.
  workers: 1,
  fullyParallel: false,
  timeout: 90_000,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  use: { trace: "retain-on-failure" },
});
