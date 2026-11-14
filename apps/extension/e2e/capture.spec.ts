// Capture and the fallback guarantee in a real Chromium, against the stub
// host: what goes to Boltget, what stays in the browser, and that the
// browser's download survives every answer but `accepted`.
import type { Page, Worker } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileByte } from "./server";
import {
  browserDownloads,
  eventually,
  expect,
  test,
  type BrowserDownload,
  type Stub,
} from "./fixtures";

/** The version the build puts in the manifest, which hello reports. */
const VERSION = (
  JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8")) as {
    version: string;
  }
).version;

const MIB = 1024 * 1024;

/** Waits for the browser to finish one download and checks its bytes. */
async function completedInBrowser(worker: Worker, url: string, size: number) {
  const item = await eventually<BrowserDownload>(async () =>
    (await browserDownloads(worker)).find((d) => d.url === url && d.state === "complete"),
  );
  const bytes = readFileSync(item.filename);
  expect(bytes.length).toBe(size);
  for (const i of [0, 1, size >> 1, size - 1]) expect(bytes[i]).toBe(fileByte(i));
  return item;
}

/** Waits until the background has registered the modifier-key script. */
async function modifierScriptReady(worker: Worker) {
  await eventually(() =>
    worker.evaluate(async () => (await chrome.scripting.getRegisteredContentScripts()).length > 0),
  );
}

async function openPage(page: Page, origin: string) {
  await page.goto(`${origin}/page.html`);
}

function noCapture(stub: Stub) {
  expect(stub.messages("capture")).toEqual([]);
}

test("says hello when installed, as the browser it runs in", async ({ worker, stub }) => {
  expect(worker.url()).toContain("background.js");
  const hello = await eventually(() => stub.messages("hello")[0]);
  expect(hello.message).toMatchObject({ v: 1, type: "hello", extension: VERSION });
  expect(["chrome", "edge"]).toContain(hello.message.browser);
});

test.describe("what goes to Boltget", () => {
  test("a big archive, with its cookies and headers; the browser's copy is dropped", async ({
    context,
    worker,
    server,
    stub,
  }) => {
    const page = await context.newPage();
    await openPage(page, server.origin);
    await page.click("#big");

    const capture = await eventually(() => stub.messages("capture")[0]);
    expect(capture.message).toMatchObject({
      v: 1,
      url: `${server.origin}/files/big.zip`,
      filename: "big.zip",
      size: 4 * MIB,
      mime: "application/zip",
      referrer: `${server.origin}/page.html`,
      incognito: false,
      forced: false,
    });
    const cookies = capture.message.cookies as { name: string; value: string }[];
    expect(cookies).toContainEqual(
      expect.objectContaining({ name: "session", value: "e2e-cookie" }),
    );
    const headers = new Map(capture.message.headers as [string, string][]);
    expect(headers.get("user-agent")).toBeTruthy();
    expect(headers.get("referer")).toBe(`${server.origin}/page.html`);

    // Cancelled and erased: no trace in the browser's list.
    await eventually(async () => (await browserDownloads(worker)).length === 0);
  });

  test("generic bytes over the size threshold", async ({ context, worker, server, stub }) => {
    const page = await context.newPage();
    await openPage(page, server.origin);
    await page.click("#generic");
    const capture = await eventually(() => stub.messages("capture")[0]);
    expect(capture.message).toMatchObject({
      url: `${server.origin}/get?id=7`,
      size: 3 * MIB,
      mime: "application/octet-stream",
    });
    await eventually(async () => (await browserDownloads(worker)).length === 0);
  });

  test("Alt+Shift forces a file the rules would leave alone", async ({
    context,
    worker,
    server,
    stub,
  }) => {
    await modifierScriptReady(worker);
    const page = await context.newPage();
    await openPage(page, server.origin);
    await page.click("#small", { modifiers: ["Alt", "Shift"] });
    const capture = await eventually(() => stub.messages("capture")[0]);
    expect(capture.message).toMatchObject({
      url: `${server.origin}/files/small.zip`,
      forced: true,
    });
  });
});

test.describe("what stays in the browser", () => {
  test("a small file", async ({ context, worker, server, stub }) => {
    const page = await context.newPage();
    await openPage(page, server.origin);
    await page.click("#small");
    await completedInBrowser(worker, `${server.origin}/files/small.zip`, 20 * 1024);
    noCapture(stub);
  });

  test("a document", async ({ context, worker, server, stub }) => {
    const page = await context.newPage();
    await openPage(page, server.origin);
    await page.click("#pdf");
    await completedInBrowser(worker, `${server.origin}/files/doc.pdf`, 3 * MIB);
    noCapture(stub);
  });

  test("a blob: download made by the page", async ({ context, worker, server, stub }) => {
    const page = await context.newPage();
    await openPage(page, server.origin);
    await page.click("#blob");
    await eventually(async () =>
      (await browserDownloads(worker)).find(
        (d) => d.url.startsWith("blob:") && d.state === "complete",
      ),
    );
    noCapture(stub);
  });

  test("a link clicked with Alt", async ({ context, worker, server, stub }) => {
    await modifierScriptReady(worker);
    const page = await context.newPage();
    await openPage(page, server.origin);
    await page.click("#big", { modifiers: ["Alt"] });
    await completedInBrowser(worker, `${server.origin}/files/big.zip`, 4 * MIB);
    noCapture(stub);
  });

  test("everything, while capture is switched off", async ({ context, worker, server, stub }) => {
    await worker.evaluate(() => chrome.storage.local.set({ captureOn: false }));
    const page = await context.newPage();
    await openPage(page, server.origin);
    await page.click("#big");
    await completedInBrowser(worker, `${server.origin}/files/big.zip`, 4 * MIB);
    noCapture(stub);
  });

  test("a site on the never list", async ({ context, worker, server, stub }) => {
    await worker.evaluate(async () => {
      const { rules } = await chrome.storage.local.get("rules");
      await chrome.storage.local.set({
        rules: { ...(rules as object), never_sites: ["127.0.0.1"] },
        rulesEdited: true,
      });
    });
    const page = await context.newPage();
    await openPage(page, server.origin);
    await page.click("#big");
    await completedInBrowser(worker, `${server.origin}/files/big.zip`, 4 * MIB);
    noCapture(stub);
  });
});

test.describe("the app's rules", () => {
  test.use({
    stubMode: {
      capture: "accept",
      rules: {
        enabled: true,
        min_size: 8 * MIB,
        extensions: ["zip"],
        never_sites: [],
        always_sites: [],
        private_windows: false,
      },
    },
  });

  test("arrive with hello and apply", async ({ context, worker, server, stub }) => {
    await eventually(() =>
      worker.evaluate(async () => {
        const { rules } = await chrome.storage.local.get("rules");
        return (rules as { min_size?: number } | undefined)?.min_size === 8 * 1024 * 1024;
      }),
    );
    const page = await context.newPage();
    await openPage(page, server.origin);
    // 4 MiB is under the app's 8 MiB.
    await page.click("#big");
    await completedInBrowser(worker, `${server.origin}/files/big.zip`, 4 * MIB);
    noCapture(stub);
  });
});

test.describe("the fallback: the browser keeps its download", () => {
  test("when the app refuses a one-time link, the browser's own response finishes", async ({
    context,
    worker,
    server,
    stub,
  }) => {
    stub.set({ capture: "reject" });
    const page = await context.newPage();
    await openPage(page, server.origin);
    await page.click("#once");
    await eventually(() => stub.messages("capture")[0]);
    await completedInBrowser(worker, `${server.origin}/files/once.zip`, 4 * MIB);
    // Resumed on its first connection, never requested again.
    expect(server.count("/files/once.zip")).toBe(1);
  });

  test("when Boltget is not installed", async ({ context, worker, server, stub }) => {
    stub.uninstall();
    const page = await context.newPage();
    await openPage(page, server.origin);
    await page.click("#big");
    await completedInBrowser(worker, `${server.origin}/files/big.zip`, 4 * MIB);
    expect(server.count("/files/big.zip")).toBe(1);
  });

  for (const capture of ["unavailable", "exit", "garbage"] as const) {
    test(`when the host answers ${capture}`, async ({ context, worker, server, stub }) => {
      stub.set({ capture });
      const page = await context.newPage();
      await openPage(page, server.origin);
      await page.click("#big");
      await eventually(() => stub.messages("capture")[0]);
      await completedInBrowser(worker, `${server.origin}/files/big.zip`, 4 * MIB);
      expect(server.count("/files/big.zip")).toBe(1);
    });
  }

  test("when the app hangs, after the timeout", async ({ context, worker, server, stub }) => {
    stub.set({ capture: "hang" });
    const page = await context.newPage();
    await openPage(page, server.origin);
    const started = Date.now();
    await page.click("#big");
    await eventually(() => stub.messages("capture")[0]);
    // Paused while the extension waits for the app.
    await eventually(async () => (await browserDownloads(worker)).find((d) => d.paused));
    await completedInBrowser(worker, `${server.origin}/files/big.zip`, 4 * MIB);
    expect(Date.now() - started).toBeGreaterThanOrEqual(15_000);
    expect(server.count("/files/big.zip")).toBe(1);
  });
});
