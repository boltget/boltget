// The real run (e2e-real/): the development build in Brave with the real
// native host and app. Local only, never in CI; see e2e-real/real.spec.ts.
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "e2e-real",
  workers: 1,
  fullyParallel: false,
  timeout: 90_000,
  retries: 0,
  reporter: "list",
});
