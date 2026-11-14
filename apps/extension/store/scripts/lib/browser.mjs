// Finds a Chromium for the captures: $BOLTGET_STORE_BROWSER, Playwright's own
// build if it is installed, or else the newest Chromium in Playwright's cache
// (a cache from another Playwright version works as well).
import { existsSync, readdirSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { chromium } from "@playwright/test";

export function chromiumPath() {
  if (process.env.BOLTGET_STORE_BROWSER) return process.env.BOLTGET_STORE_BROWSER;
  const own = chromium.executablePath();
  if (existsSync(own)) return own;
  const cache =
    process.env.PLAYWRIGHT_BROWSERS_PATH ??
    (process.platform === "win32"
      ? join(process.env.LOCALAPPDATA ?? join(homedir(), "AppData", "Local"), "ms-playwright")
      : process.platform === "darwin"
        ? join(homedir(), "Library", "Caches", "ms-playwright")
        : join(homedir(), ".cache", "ms-playwright"));
  const builds = existsSync(cache)
    ? readdirSync(cache)
        .filter((d) => /^chromium-\d+$/.test(d))
        .sort((a, b) => Number(b.split("-")[1]) - Number(a.split("-")[1]))
    : [];
  const binary =
    process.platform === "win32"
      ? ["chrome-win64", "chrome.exe"]
      : process.platform === "darwin"
        ? ["chrome-mac", "Chromium.app", "Contents", "MacOS", "Chromium"]
        : ["chrome-linux", "chrome"];
  for (const build of builds) {
    const path = join(cache, build, ...binary);
    if (existsSync(path)) return path;
  }
  throw new Error(
    "No Chromium found: run `pnpm exec playwright install chromium` in apps/extension",
  );
}
