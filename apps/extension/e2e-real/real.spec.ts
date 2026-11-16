// The real run: the development build of the extension in a real Chromium
// browser (Brave by default), the debug native host registered with
// `--register`, and the debug desktop app it starts, all on scratch data.
// Local only, never in CI: `pnpm --filter @boltget/extension e2e:real`
// (docs/BROWSER-INTEGRATION.md, "Testing").
//
// Needs `cargo build -p boltget-native-host -p boltget-desktop -p boltget-cli`
// and `target\debug\boltget-native-host.exe --register`. The app's page
// comes from the desktop dev server (port 1420), started here when it is not
// running. The tests run in order and share one browser and one app.
import {
  chromium,
  test,
  expect,
  type Browser,
  type BrowserContext,
  type Page,
  type Worker,
} from "@playwright/test";
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import extensions from "../../../packages/native-messaging/extensions.json" with { type: "json" };
import { MIB, expected, startRealServer, type RealServer } from "./server";

const EXTENSION_ID = extensions.development.chromium[0] ?? "";
const repo = fileURLToPath(new URL("../../../", import.meta.url));
const devBuild = join(repo, "apps/extension/.output/chrome-mv3-dev");
/** The e2e flavor: the same code with site access required, on its own host name. */
const e2eBuild = join(repo, "apps/extension/.output/chrome-mv3-e2e");
const E2E_HOST_KEY = [
  "HKCU",
  "Software",
  "Google",
  "Chrome",
  "NativeMessagingHosts",
  "app.boltget.native_host_e2e",
].join("\\");
const debug = join(repo, "target/debug");
const BROWSER =
  process.env.BOLTGET_REAL_BROWSER ??
  "C:\\Program Files\\BraveSoftware\\Brave-Browser\\Application\\brave.exe";
const CDP_PORT = 9339;
const APP_EXE = "boltget-desktop.exe";

test.describe.configure({ mode: "serial" });

let root: string;
let dataDir: string;
let appFolder: string;
let browserFolder: string;
let server: RealServer;
let context: BrowserContext;
let worker: Worker;
let page: Page;
let app: Browser | null = null;
let appPage: Page | null = null;
let vite: ChildProcess | null = null;

// ---------------------------------------------------------------- helpers

function appRunning(): boolean {
  const out = execFileSync("tasklist", ["/FI", `IMAGENAME eq ${APP_EXE}`, "/NH"], {
    encoding: "utf8",
  });
  return out.includes(APP_EXE);
}

async function eventually<T>(
  check: () => Promise<T | null | undefined | false> | T | null | undefined | false,
  timeout = 30_000,
  what = "condition",
): Promise<T> {
  const end = Date.now() + timeout;
  for (;;) {
    const value = await check();
    if (value) return value;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 200));
  }
}

async function reachable(url: string): Promise<boolean> {
  try {
    await fetch(url, { signal: AbortSignal.timeout(1000) });
    return true;
  } catch {
    return false;
  }
}

/** The app's page over WebView2's DevTools port, once the app is up. */
async function appWindow(): Promise<Page> {
  if (appPage && !appPage.isClosed()) return appPage;
  await eventually(
    () => reachable(`http://127.0.0.1:${CDP_PORT}/json/version`),
    30_000,
    "app DevTools",
  );
  app = await chromium.connectOverCDP(`http://127.0.0.1:${CDP_PORT}`);
  appPage = await eventually(
    async () => {
      for (const c of app?.contexts() ?? [])
        for (const p of c.pages())
          if (await p.evaluate(() => "__boltget" in window).catch(() => false)) return p;
      return null;
    },
    30_000,
    "the app's page",
  );
  return appPage;
}

interface Row {
  id: number;
  state: string;
  url: string;
  fileName: string | null;
  path: string | null;
  total: number | null;
  downloaded: number;
  private: boolean;
  error: unknown;
}

/** Runs `__boltget.commands[name](...args)` in the app; unwraps results. */
async function command<T>(name: string, ...args: unknown[]): Promise<T> {
  const p = await appWindow();
  const out = await p.evaluate(
    async ([name, args]) => {
      const commands = (
        window as unknown as {
          __boltget: { commands: Record<string, (...a: unknown[]) => Promise<unknown>> };
        }
      ).__boltget.commands;
      return await commands[name as string]!(...(args as unknown[]));
    },
    [name, args] as const,
  );
  if (out && typeof out === "object" && "status" in out) {
    const r = out as { status: string; data?: T; error?: unknown };
    if (r.status !== "ok") throw new Error(`${name}: ${JSON.stringify(r.error)}`);
    return r.data as T;
  }
  return out as T;
}

const rows = () => command<Row[]>("listDownloads");
const row = async (name: string) => (await rows()).find((r) => r.fileName === name);

async function completed(name: string, timeout = 60_000): Promise<Row> {
  return eventually(
    async () => {
      const r = await row(name);
      if (r && ["failed", "cancelled", "needs_new_link"].includes(r.state))
        throw new Error(`${name} is ${r.state}: ${JSON.stringify(r.error)}`);
      return r?.state === "completed" ? r : null;
    },
    timeout,
    `${name} to complete in the app`,
  );
}

function sameBytes(path: string, size: number): boolean {
  const got = readFileSync(path);
  return got.length === size && got.equals(expected(size));
}

interface Item {
  id: number;
  url: string;
  finalUrl: string;
  state: string;
  filename: string;
  exists: boolean;
  totalBytes: number;
}

async function browserDownloads(): Promise<Item[]> {
  return worker.evaluate(async () =>
    (await chrome.downloads.search({})).map((d) => ({
      id: d.id,
      url: d.url,
      finalUrl: d.finalUrl,
      state: d.state,
      filename: d.filename,
      exists: d.exists,
      totalBytes: d.totalBytes,
    })),
  );
}

const inBrowser = async (path: string) =>
  (await browserDownloads()).filter((d) => d.url.includes(path) || d.finalUrl.includes(path));

/** Opens a page of links and clicks the one to `href`. */
async function click(href: string): Promise<void> {
  await page.goto(`${server.origin}/page.html?links=${encodeURIComponent(href)}`);
  await page.click("#l0");
}

/** The extension's service worker (it restarts when its settings change). */
async function extensionWorker(): Promise<Worker> {
  return (
    context.serviceWorkers().find((w) => w.url().includes(EXTENSION_ID)) ??
    (await context.waitForEvent("serviceworker", {
      predicate: (w) => w.url().includes(EXTENSION_ID),
      timeout: 15_000,
    }))
  );
}

/** Changes the extension's own configuration as the extensions page would. */
async function configureExtension(config: Record<string, unknown>): Promise<void> {
  const tab = await context.newPage();
  await tab.goto(`chrome://extensions/?id=${EXTENSION_ID}`);
  await tab.evaluate(
    async (config) => {
      const dev = (
        chrome as unknown as {
          developerPrivate: {
            updateProfileConfiguration(c: unknown): Promise<void>;
            updateExtensionConfiguration(c: unknown): Promise<void>;
          };
        }
      ).developerPrivate;
      // Without developer mode, the reload that follows disables an unpacked
      // extension.
      await dev.updateProfileConfiguration({ inDeveloperMode: true });
      await dev.updateExtensionConfiguration(config);
    },
    { extensionId: EXTENSION_ID, ...config },
  );
  await tab.close();
}

/** Starts the browser with `build` loaded, on a fresh profile in the run's folder. */
async function launch(
  build: string,
  name: string,
): Promise<{ context: BrowserContext; page: Page }> {
  const profile = join(root, name);
  mkdirSync(join(profile, "Default"), { recursive: true });
  writeFileSync(
    join(profile, "Default", "Preferences"),
    JSON.stringify({
      download: { default_directory: browserFolder, prompt_for_download: false },
      savefile: { default_directory: browserFolder },
    }),
  );
  const context = await chromium.launchPersistentContext(profile, {
    headless: false,
    executablePath: BROWSER,
    args: [
      "--headless=new",
      "--no-first-run",
      `--disable-extensions-except=${build}`,
      `--load-extension=${build}`,
    ],
    // The host and the app it starts inherit these.
    env: {
      ...process.env,
      BOLTGET_DATA_DIR: dataDir,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${CDP_PORT}`,
    },
  });
  const page = context.pages()[0] ?? (await context.newPage());
  // Playwright takes over downloads, which skips onDeterminingFilename.
  const cdp = await context.newCDPSession(page);
  await cdp.send("Browser.setDownloadBehavior", { behavior: "default" });
  return { context, page };
}

// ------------------------------------------------------------------ setup

test.beforeAll(async () => {
  test.setTimeout(120_000);
  for (const exe of ["boltget-native-host.exe", APP_EXE, "boltget.exe"])
    if (!existsSync(join(debug, exe))) throw new Error(`build ${exe} first (see the header)`);
  const status = execFileSync(join(debug, "boltget-native-host.exe"), ["--status"], {
    encoding: "utf8",
  });
  if (!/Chrome: Registered/.test(status))
    throw new Error(`register the development host first:\n${status}`);
  try {
    execFileSync("reg", ["query", E2E_HOST_KEY], { stdio: "ignore" });
    throw new Error("the e2e host name is registered (an e2e run in progress?)");
  } catch (e) {
    if (e instanceof Error && e.message.startsWith("the e2e")) throw e;
  }
  if (appRunning())
    throw new Error(`${APP_EXE} is running; quit it first (the run starts its own)`);

  root = mkdtempSync(join(tmpdir(), "boltget-real-"));
  dataDir = join(root, "data");
  appFolder = join(root, "app-downloads");
  browserFolder = join(root, "browser-downloads");
  for (const d of [dataDir, appFolder, browserFolder]) mkdirSync(d, { recursive: true });

  // A scratch database whose downloads go to the run's folder.
  execFileSync(join(debug, "boltget.exe"), ["config", "unset", "limit"], {
    env: { ...process.env, BOLTGET_DATA_DIR: dataDir },
    stdio: "ignore",
  });
  const db = new DatabaseSync(join(dataDir, "boltget.db"));
  const put = db.prepare("INSERT OR REPLACE INTO setting (key, value) VALUES (?, ?)");
  put.run("app.download_folder", JSON.stringify(appFolder));
  put.run("app.notifications", "false");
  db.close();

  if (!(await reachable("http://localhost:1420"))) {
    vite = spawn("pnpm", ["--filter", "@boltget/desktop", "dev"], {
      cwd: repo,
      shell: true,
      stdio: "ignore",
    });
    await eventually(() => reachable("http://localhost:1420"), 60_000, "the desktop dev server");
  }

  server = await startRealServer();

  ({ context, page } = await launch(devBuild, "profile"));
  worker = await extensionWorker();
});

test.afterAll(async () => {
  if (appPage) await command("quitApp").catch(() => {});
  await app?.close().catch(() => {});
  await context?.close().catch(() => {});
  await server?.close();
  // Give the app time to quit before its folder goes.
  await eventually(() => !appRunning(), 15_000, "the app to quit").catch(() => {
    execFileSync("taskkill", ["/IM", APP_EXE, "/F"], { stdio: "ignore" });
  });
  vite?.kill();
  if (root && !process.env.BOLTGET_REAL_KEEP)
    rmSync(root, { recursive: true, force: true, maxRetries: 5 });
});

// ------------------------------------------------------------------ tests

test("cold start: the host starts the app, a 4 MiB archive goes to it", async () => {
  expect(appRunning()).toBe(false);
  await click("/files/big.zip");
  const done = await completed("big.zip");
  expect(appRunning()).toBe(true);
  expect(done.path).toBe(join(appFolder, "big.zip"));
  expect(sameBytes(done.path!, 4 * MIB)).toBe(true);
  // The browser's copy is cancelled and erased.
  await eventually(
    async () => (await inBrowser("/files/big.zip")).length === 0,
    10_000,
    "the browser's copy to go",
  );
  expect(existsSync(join(browserFolder, "big.zip"))).toBe(false);
});

test("a small file and a PDF stay in the browser", async () => {
  for (const [path, name, size] of [
    ["/files/small.zip", "small.zip", 20 * 1024],
    ["/files/doc.pdf", "doc.pdf", 3 * MIB],
  ] as const) {
    await click(path);
    const item = await eventually(
      async () => (await inBrowser(path)).find((d) => d.state === "complete"),
      20_000,
      `${name} in the browser`,
    );
    expect(sameBytes(item.filename, size)).toBe(true);
    expect(await row(name)).toBeUndefined();
  }
});

test("a cookie-protected file: without site access it stays, with it the app gets the cookie", async () => {
  // Without site access the extension sends no cookies: the app's first
  // request is refused and the browser keeps the download.
  await click("/files/locked.zip");
  const item = await eventually(
    async () => (await inBrowser("/files/locked.zip")).find((d) => d.state === "complete"),
    30_000,
    "locked.zip in the browser",
  );
  expect(sameBytes(item.filename, 4 * MIB)).toBe(true);
  expect(await row("locked.zip")).toBeUndefined();

  // Site access is an optional permission: the browser's prompt cannot be
  // answered from a test, nor granted from the extensions page. The e2e
  // flavor (same code, site access required) is started instead, with its
  // host name pointing at the real host for the length of this test.
  const manifest = join(root, "e2e-host.json");
  writeFileSync(
    manifest,
    JSON.stringify({
      name: "app.boltget.native_host_e2e",
      description: "Boltget real run",
      path: join(debug, "boltget-native-host.exe"),
      type: "stdio",
      allowed_origins: [`chrome-extension://${EXTENSION_ID}/`],
    }),
  );
  execFileSync("reg", ["add", E2E_HOST_KEY, "/ve", "/t", "REG_SZ", "/d", manifest, "/f"], {
    stdio: "ignore",
  });
  const first = { context, page, worker };
  try {
    ({ context, page } = await launch(e2eBuild, "profile-e2e"));
    worker = await extensionWorker();
    expect(
      await worker.evaluate(() => chrome.permissions.contains({ origins: ["<all_urls>"] })),
    ).toBe(true);
    await click("/files/locked2.zip");
    const done = await completed("locked2.zip");
    expect(sameBytes(done.path!, 4 * MIB)).toBe(true);
    // Every request of the app carried the browser's cookie.
    expect(server.cookies("/files/locked2.zip").every((c) => c.includes("session=real-run"))).toBe(
      true,
    );
  } finally {
    await context.close().catch(() => {});
    ({ context, page, worker } = first);
    execFileSync("reg", ["delete", E2E_HOST_KEY, "/f"], { stdio: "ignore" });
  }
});

test("a one-time link is refused by the app and finishes in the browser", async () => {
  await click("/files/once.zip");
  const item = await eventually(
    async () => (await inBrowser("/files/once.zip")).find((d) => d.state === "complete"),
    30_000,
    "once.zip in the browser",
  );
  expect(sameBytes(item.filename, 4 * MIB)).toBe(true);
  expect(server.count("/files/once.zip")).toBe(2); // the browser's, the app's refused one
  expect(await row("once.zip")).toBeUndefined();
});

test("a redirect to a Content-Disposition name", async () => {
  await click("/redir/archive");
  const done = await completed("named-archive.zip");
  expect(done.path).toBe(join(appFolder, "named-archive.zip"));
  expect(sameBytes(done.path!, 5 * MIB)).toBe(true);
  await eventually(
    async () => (await inBrowser("/cd/blob")).length === 0,
    10_000,
    "the browser's copy to go",
  );
});

/** Captures an expiring link and waits until the app needs a new one. */
async function expireMidway(name: string): Promise<Row> {
  await click(server.expiring(name, 2500));
  let last: unknown;
  const r = await eventually(
    async () => {
      const r = await row(name);
      last = { app: r, browser: await inBrowser(name) };
      return r?.state === "needs_new_link" ? r : null;
    },
    60_000,
    `${name} to need a new link`,
  ).catch((e: Error) => {
    throw new Error(`${e.message}: ${JSON.stringify(last)}`);
  });
  expect(r.downloaded).toBeGreaterThan(0);
  expect(r.downloaded).toBeLessThan(24 * MIB);
  return r;
}

test("link refresh: a fresh link from the browser resumes the download", async () => {
  test.setTimeout(150_000);
  const stuck = await expireMidway("disk.iso");
  const fresh = server.expiring("disk.iso");
  await click(fresh);
  const offer = await eventually(
    async () =>
      (
        await command<{ offer: number; id: number; name: string; percent: number }[]>(
          "resumeOffers",
        )
      )[0],
    30_000,
    "the resume offer",
  );
  expect(offer.id).toBe(stuck.id);
  expect(offer.name).toBe("disk.iso");
  // The app took the capture before the user answered: the browser's copy goes.
  await eventually(
    async () => (await inBrowser("disk.iso")).length === 0,
    10_000,
    "the browser's copy to go",
  );
  const answer = await command<{ outcome: string }>("answerResumeOffer", offer.offer, true);
  expect(answer.outcome).toBe("resumed");
  const done = await completed("disk.iso", 90_000);
  expect(done.id).toBe(stuck.id);
  // The new link replaced the old one.
  expect(done.url).toBe(fresh);
  expect(sameBytes(done.path!, 24 * MIB)).toBe(true);
  expect((await rows()).filter((r) => r.fileName?.startsWith("disk")).length).toBe(1);
});

test("link refresh: declining adds the fresh link as a new download", async () => {
  test.setTimeout(150_000);
  const stuck = await expireMidway("decline.iso");
  await click(server.expiring("decline.iso"));
  const offer = await eventually(
    async () => (await command<{ offer: number; id: number }[]>("resumeOffers"))[0],
    30_000,
    "the resume offer",
  );
  expect(offer.id).toBe(stuck.id);
  const answer = await command<{ outcome: string }>("answerResumeOffer", offer.offer, false);
  expect(answer.outcome).toBe("added");
  const fresh = await eventually(
    async () => (await rows()).find((r) => r.id !== stuck.id && r.fileName?.startsWith("decline")),
    10_000,
    "the new download",
  );
  const done = await completed(fresh.fileName!, 90_000);
  expect(sameBytes(done.path!, 24 * MIB)).toBe(true);
  expect((await rows()).find((r) => r.id === stuck.id)?.state).toBe("needs_new_link");
});

test("capture rules changed in the app reach the next download", async () => {
  test.setTimeout(120_000);
  await command("updateSettings", { capture: { minSize: 8 * MIB } });
  // The extension asks again when its last hello is over 30 s old.
  await new Promise((r) => setTimeout(r, 31_000));
  await click("/files/rules.zip");
  const item = await eventually(
    async () => (await inBrowser("/files/rules.zip")).find((d) => d.state === "complete"),
    30_000,
    "rules.zip in the browser",
  );
  expect(sameBytes(item.filename, 4 * MIB)).toBe(true);
  expect(await row("rules.zip")).toBeUndefined();
  await command("updateSettings", { capture: { minSize: MIB } });
});

test("a private window with Forget private-window downloads on", async () => {
  test.setTimeout(120_000);
  await command("updateSettings", { capture: { privateWindows: true }, forgetPrivate: true });
  // "Allow in Private" on the extensions page; the extension reloads.
  await configureExtension({ incognitoAccess: true });
  // Its options page wakes the new service worker, and its hello brings the
  // app's rules with Private windows on.
  const options = await context.newPage();
  const waking = extensionWorker();
  await eventually(
    () =>
      options.goto(`chrome-extension://${EXTENSION_ID}/options.html`).then(
        () => true,
        () => false,
      ),
    15_000,
    "the reloaded extension",
  );
  worker = await waking;
  await options.waitForTimeout(1000);
  await options.close();
  expect(await worker.evaluate(() => chrome.extension.isAllowedIncognitoAccess())).toBe(true);
  const url = `${server.origin}/files/private.zip`;
  await worker.evaluate((url) => chrome.windows.create({ incognito: true, url }), url);
  const file = join(appFolder, "private.zip");
  await eventually(
    () => existsSync(file) && readFileSync(file).length === 4 * MIB,
    60_000,
    "private.zip in the app's folder",
  );
  expect(sameBytes(file, 4 * MIB)).toBe(true);
  // Forgotten: not in the app's list once complete.
  await eventually(
    async () => !(await row("private.zip")),
    15_000,
    "private.zip to leave the list",
  );
  expect(readdirSync(browserFolder)).not.toContain("private.zip");
});
