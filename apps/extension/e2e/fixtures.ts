// Playwright fixtures: a fresh browser profile with the e2e build of the
// extension loaded unpacked, the stub host registered for it, and the local
// test server. Each test gets its own of each.
import { test as base, chromium, type BrowserContext, type Worker } from "@playwright/test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import extensions from "../../../packages/native-messaging/extensions.json" with { type: "json" };
import { registerStubHost } from "./host";
import { startServer, type TestServer } from "./server";

/** The pinned development ID; the e2e build carries the development key. */
export const EXTENSION_ID = extensions.development.chromium[0] ?? "";

const extensionPath = fileURLToPath(new URL("../.output/chrome-mv3-e2e", import.meta.url));

/** What the stub answers (see stub-host.mjs). */
export interface StubMode {
  capture?: "accept" | "reject" | "unavailable" | "hang" | "exit" | "garbage";
  app?: string | null;
  rules?: unknown;
}

/** One message the stub received. */
export interface Logged {
  at: number;
  message: Record<string, unknown> & { type: string };
}

export interface Stub {
  dir: string;
  set(mode: StubMode): void;
  messages(type?: string): Logged[];
  /** Makes the host vanish, as if Boltget were not installed. */
  uninstall(): void;
}

/** A download as `chrome.downloads.search` reports it. */
export interface BrowserDownload {
  id: number;
  url: string;
  state: string;
  paused: boolean;
  error?: string;
  bytesReceived: number;
  totalBytes: number;
  filename: string;
  exists: boolean;
}

interface Fixtures {
  /** What the stub answers from the start (the install's hello included). */
  stubMode: StubMode;
  server: TestServer;
  stub: Stub;
  context: BrowserContext;
  worker: Worker;
}

export const test = base.extend<Fixtures>({
  stubMode: [{ capture: "accept" }, { option: true }],

  // eslint-disable-next-line no-empty-pattern -- Playwright's fixture signature
  server: async ({}, use) => {
    const server = await startServer();
    await use(server);
    await server.close();
  },

  stub: async ({ stubMode }, use) => {
    const dir = mkdtempSync(join(tmpdir(), "boltget-stub-"));
    let registration: ReturnType<typeof registerStubHost> | null = null;
    const stub: Stub = {
      dir,
      set: (mode) => writeFileSync(join(dir, "mode.json"), JSON.stringify(mode)),
      messages(type) {
        let text: string;
        try {
          text = readFileSync(join(dir, "log.jsonl"), "utf8");
        } catch {
          return [];
        }
        const all = text
          .split("\n")
          .filter(Boolean)
          .map((line) => JSON.parse(line) as Logged);
        return type ? all.filter((l) => l.message.type === type) : all;
      },
      uninstall: () => registration?.unregister(),
    };
    stub.set(stubMode);
    const profile = join(dir, "profile");
    registration = registerStubHost(EXTENSION_ID, join(dir, "host"), profile);
    await use(stub);
    registration.unregister();
    rmSync(dir, { recursive: true, force: true, maxRetries: 5 });
  },

  context: async ({ stub }, use) => {
    // The browser's own downloads go to the test's folder, never the user's.
    const profile = join(stub.dir, "profile");
    const downloads = join(stub.dir, "downloads");
    mkdirSync(join(profile, "Default"), { recursive: true });
    writeFileSync(
      join(profile, "Default", "Preferences"),
      JSON.stringify({
        download: { default_directory: downloads, prompt_for_download: false },
        savefile: { default_directory: downloads },
      }),
    );
    const context = await chromium.launchPersistentContext(profile, {
      // New headless mode runs extensions; Playwright's own flag would pick
      // the headless shell, which does not.
      headless: false,
      executablePath: process.env.BOLTGET_E2E_BROWSER || undefined,
      args: [
        "--headless=new",
        `--disable-extensions-except=${extensionPath}`,
        `--load-extension=${extensionPath}`,
      ],
      env: { ...process.env, BOLTGET_STUB_DIR: stub.dir },
    });
    // Playwright takes over downloads (`allowAndName`), which skips the
    // extensions' onDeterminingFilename: give them back to the browser.
    const page = context.pages()[0] ?? (await context.newPage());
    const cdp = await context.newCDPSession(page);
    await cdp.send("Browser.setDownloadBehavior", { behavior: "default" });
    await use(context);
    await context.close();
  },

  worker: async ({ context }, use) => {
    const worker =
      context.serviceWorkers().find((w) => w.url().includes(EXTENSION_ID)) ??
      (await context.waitForEvent("serviceworker", {
        predicate: (w) => w.url().includes(EXTENSION_ID),
      }));
    await use(worker);
  },
});

export const expect = test.expect;

/** The browser's downloads, from the extension's service worker. */
export function browserDownloads(worker: Worker): Promise<BrowserDownload[]> {
  return worker.evaluate(async () => {
    const items = await chrome.downloads.search({});
    return items.map((d) => ({
      id: d.id,
      url: d.url,
      state: d.state,
      paused: d.paused,
      error: d.error,
      bytesReceived: d.bytesReceived,
      totalBytes: d.totalBytes,
      filename: d.filename,
      exists: d.exists,
    }));
  });
}

/** Waits until `check` returns something truthy. */
export async function eventually<T>(
  check: () => Promise<T | null | undefined | false> | T | null | undefined | false,
  timeout = 30_000,
): Promise<T> {
  const end = Date.now() + timeout;
  for (;;) {
    const value = await check();
    if (value) return value;
    if (Date.now() > end) throw new Error("timed out waiting");
    await new Promise((r) => setTimeout(r, 100));
  }
}
