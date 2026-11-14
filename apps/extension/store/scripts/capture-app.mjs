// Captures the real desktop app UI for the store images: the app's own Svelte
// UI from `pnpm --filter @boltget/desktop dev`, which outside Tauri runs on
// the app's simulated engine (apps/desktop/src/lib/engine/mock.ts). The
// downloads, their progress, the file map and the connections are the app's
// real components; only the engine behind them is simulated.
//
// Starts the dev server when none answers on its port, and stops it again.
import { spawn } from "node:child_process";
import { join } from "node:path";
import { chromium } from "@playwright/test";
import { chromiumPath } from "./lib/browser.mjs";
import { CAPTURES, REPO_DIR, SCALE, savePng } from "./lib/paths.mjs";

const URL_BASE = "http://localhost:1420/";
/** Six downloads: two running, one paused, three finished (see mock.ts). */
const SEED = "?mock=6&active=2";
const VIEWPORT = { width: 1080, height: 680 };
/** How long the simulated downloads run before the capture, for a full speed graph. */
const RUN_MS = 7_000;

async function answering() {
  try {
    return (await fetch(URL_BASE)).ok;
  } catch {
    return false;
  }
}

async function startServer() {
  if (await answering()) return () => {};
  const child = spawn("pnpm --filter @boltget/desktop dev", {
    cwd: REPO_DIR,
    stdio: "ignore",
    shell: true,
  });
  const end = Date.now() + 60_000;
  while (!(await answering())) {
    if (Date.now() > end) throw new Error("the desktop dev server did not start");
    await new Promise((r) => setTimeout(r, 500));
  }
  return () => {
    if (process.platform === "win32") {
      spawn("taskkill", ["/pid", String(child.pid), "/t", "/f"], { stdio: "ignore" });
    } else child.kill();
  };
}

async function openApp(browser, scheme) {
  const page = await browser.newPage({
    viewport: VIEWPORT,
    deviceScaleFactor: SCALE,
    colorScheme: scheme,
    locale: "en-US",
  });
  await page.goto(`${URL_BASE}${SEED}&theme=${scheme}`);
  await page.getByText("ubuntu-24.04.1-desktop-amd64.iso").first().waitFor();
  await page.evaluate(() => document.fonts.ready);
  return page;
}

/** Writes the app captures: the running list with the details pane, and the paused download. */
export async function captureApp() {
  const stop = await startServer();
  const browser = await chromium.launch({
    executablePath: chromiumPath(),
    args: ["--hide-scrollbars"],
  });
  try {
    const pages = {};
    for (const scheme of ["dark", "light"]) {
      const page = await openApp(browser, scheme);
      await page.getByText("ubuntu-24.04.1-desktop-amd64.iso").first().click();
      pages[scheme] = page;
    }
    await new Promise((r) => setTimeout(r, RUN_MS));
    for (const [scheme, page] of Object.entries(pages)) {
      // Nothing hovered, so no row shows its hover actions but the selected one.
      await page.mouse.move(VIEWPORT.width - 2, VIEWPORT.height - 2);
      await page.waitForTimeout(300);
      await savePng(await page.screenshot(), join(CAPTURES, `app-running-${scheme}.png`));

      await page.getByText("big_buck_bunny_1080p_h264.mov").first().click();
      await page.mouse.move(VIEWPORT.width - 2, VIEWPORT.height - 2);
      await page.waitForTimeout(800);
      await savePng(await page.screenshot(), join(CAPTURES, `app-paused-${scheme}.png`));
      await savePng(
        await page.locator(".pane").first().screenshot(),
        join(CAPTURES, `app-paused-pane-${scheme}.png`),
      );
      await page.close();
    }
  } finally {
    await browser.close();
    stop();
  }
}
