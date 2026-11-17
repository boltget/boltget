// Media detection and the store policy in a real Chromium, against the stub
// host (docs/BROWSER-INTEGRATION.md, "Media detection panel" and "Store policy
// constraints"). The e2e build applies the store policy like every build, so
// these check what the stores get: on YouTube's sites nothing is noticed, no
// badge, no popup entry, and no `media_page` is ever sent; on another site
// with a video, the popup sends one with the page's hints.
//
// The pages are served by Playwright's routing under their real names
// (`https://www.youtube.com/...`, `https://video.example/...`): no request
// leaves the machine, and the browser and the extension see the real
// addresses.
import type { BrowserContext, Page, Route, Worker } from "@playwright/test";
import { EXTENSION_ID, eventually, expect, test, type Stub } from "./fixtures";

const popup = `chrome-extension://${EXTENSION_ID}/popup.html`;

const MIB = 1024 * 1024;

/** A page with a video element, a declared video, and a stream it loads. */
function videoPage(cdn: string): string {
  return `<!doctype html>
<html><head><meta charset="utf-8"><title>A clip</title>
<meta property="og:video" content="${cdn}/media/clip.mp4">
</head><body>
<video id="v" src="${cdn}/media/clip.mp4" preload="metadata" muted></video>
<script>
  fetch("${cdn}/media/720p/master.m3u8").then((r) => r.text()).then(() => {
    document.title = "A clip (loaded)";
  });
</script>
</body></html>`;
}

const PLAYLIST = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000\nlow.m3u8\n";

/** Serves `host`'s pages and media, with the CORS a cross-site player gets. */
async function serve(context: BrowserContext, origin: string, cdn: string) {
  await context.route(`${origin}/**`, (route: Route) =>
    route.fulfill({
      status: 200,
      contentType: "text/html; charset=utf-8",
      headers: { "set-cookie": "sid=page-cookie; Path=/; SameSite=Lax; Secure" },
      body: videoPage(cdn),
    }),
  );
  await context.route(`${cdn}/media/**`, (route: Route) => {
    const url = route.request().url();
    const cors = { "access-control-allow-origin": "*" };
    if (url.endsWith(".m3u8")) {
      return route.fulfill({
        status: 200,
        contentType: "application/vnd.apple.mpegurl",
        headers: cors,
        body: PLAYLIST,
      });
    }
    return route.fulfill({
      status: 200,
      contentType: "video/mp4",
      headers: cors,
      body: Buffer.alloc(MIB),
    });
  });
}

/** The tab showing `url`, from the service worker. */
async function tabOf(worker: Worker, url: string): Promise<number> {
  return eventually(() =>
    worker.evaluate(async (u) => {
      const tabs = await chrome.tabs.query({});
      return tabs.find((t) => t.url?.startsWith(u))?.id ?? null;
    }, url),
  );
}

async function sessionMedia(worker: Worker, tabId: number) {
  return worker.evaluate(async (id) => {
    const key = `media:${id}`;
    return ((await chrome.storage.session.get(key))[key] ?? null) as {
      page: string | null;
      hints: { url: string; kind: string }[];
    } | null;
  }, tabId);
}

async function badge(worker: Worker, tabId: number): Promise<string> {
  return worker.evaluate((id) => chrome.action.getBadgeText({ tabId: id }), tabId);
}

async function openPopup(context: BrowserContext, tabId: number): Promise<Page> {
  const page = await context.newPage();
  await page.goto(`${popup}?tab=${tabId}`);
  await expect(page.getByRole("status").first()).toContainText("Connected to Boltget");
  return page;
}

function noMediaPage(stub: Stub) {
  expect(stub.messages("media_page")).toEqual([]);
}

test("on another site, the popup offers the page's video and sends media_page", async ({
  context,
  worker,
  stub,
}) => {
  await serve(context, "https://video.example", "https://cdn.video.example");
  const page = await context.newPage();
  await page.goto("https://video.example/watch?v=1");
  await expect(page).toHaveTitle("A clip (loaded)");
  const tabId = await tabOf(worker, "https://video.example/");

  // The stream the page loaded is noticed; the badge counts it.
  const seen = await eventually(async () => {
    const s = await sessionMedia(worker, tabId);
    return s?.hints.some((h) => h.kind === "hls") ? s : null;
  });
  expect(seen.hints.map((h) => h.url)).toContain(
    "https://cdn.video.example/media/720p/master.m3u8",
  );
  await eventually(async () => (await badge(worker, tabId)) !== "");

  const panel = await openPopup(context, tabId);
  await expect(panel.getByText("Video on this page")).toBeVisible();
  await expect(panel.getByText("HLS stream")).toBeVisible();
  await expect(panel.getByText("720p · cdn.video.example")).toBeVisible();
  await panel.getByRole("button", { name: "Download video from this page" }).click();
  await expect(panel.getByText("Opened in Boltget. Choose the quality there.")).toBeVisible();

  const sent = await eventually(() => stub.messages("media_page")[0]);
  expect(sent.message).toMatchObject({
    v: 2,
    type: "media_page",
    page_url: "https://video.example/watch?v=1",
    title: "A clip (loaded)",
    incognito: false,
  });
  const message = sent.message as unknown as {
    hints: { url: string; kind: string }[];
    headers: [string, string][];
    cookies: { name: string; value: string }[];
  };
  expect(message.hints).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        url: "https://cdn.video.example/media/720p/master.m3u8",
        kind: "hls",
      }),
      // The page's own video element and declared video, from the popup's look.
      { url: "https://cdn.video.example/media/clip.mp4", kind: "file" },
    ]),
  );
  const headers = Object.fromEntries(message.headers);
  expect(headers.referer).toBe("https://video.example/watch?v=1");
  expect(headers["user-agent"]).toBeTruthy();
  expect(message.cookies).toEqual(
    expect.arrayContaining([expect.objectContaining({ name: "sid", value: "page-cookie" })]),
  );

  // Choosing one hint sends that one alone.
  await panel.reload();
  await panel
    .getByRole("button", { name: "Send the hls stream from cdn.video.example to Boltget" })
    .click();
  const second = await eventually(() => stub.messages("media_page")[1]);
  expect((second.message as unknown as { hints: unknown[] }).hints).toEqual([
    expect.objectContaining({ url: "https://cdn.video.example/media/720p/master.m3u8" }),
  ]);

  // Moving to another page clears the tab.
  await page.goto("https://video.example/other");
  await eventually(async () => {
    const s = await sessionMedia(worker, tabId);
    return s?.page === "https://video.example/other";
  });
});

test.describe("an app from before video", () => {
  test.use({ stubMode: { capture: "accept", media: "unsupported" } });

  test("the popup says to update Boltget", async ({ context, worker, stub }) => {
    await serve(context, "https://video.example", "https://cdn.video.example");
    const page = await context.newPage();
    await page.goto("https://video.example/watch?v=2");
    const tabId = await tabOf(worker, "https://video.example/");
    const panel = await openPopup(context, tabId);
    await panel.getByRole("button", { name: "Download video from this page" }).click();
    await expect(
      panel.getByText("Update Boltget, then try again.", { exact: false }),
    ).toBeVisible();
    expect(stub.messages("media_page")).toHaveLength(1);
  });
});

for (const site of [
  "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
  "https://music.youtube.com/watch?v=1",
  "https://youtu.be/abc",
  "https://www.youtube-nocookie.com/embed/abc",
]) {
  test(`the store policy: nothing on ${new URL(site).hostname}`, async ({
    context,
    worker,
    stub,
  }) => {
    const origin = new URL(site).origin;
    // The media come from a host the policy does not name: only the page
    // excludes them.
    await serve(context, origin, "https://cdn.video.example");
    const page = await context.newPage();
    await page.goto(site);
    await expect(page).toHaveTitle("A clip (loaded)");
    const tabId = await tabOf(worker, origin);

    // The same page elsewhere is noticed (the first test); here, after its
    // stream and video loaded, nothing was.
    await page.waitForTimeout(500);
    const seen = await sessionMedia(worker, tabId);
    // The extension watched the tab (it knows its page) and kept nothing.
    expect(seen?.page).toBe(site);
    expect(seen?.hints ?? []).toEqual([]);
    expect(await badge(worker, tabId)).toBe("");

    // The popup has no media entry and does not look into the page.
    const panel = await openPopup(context, tabId);
    await expect(panel.getByText("Send downloads to Boltget")).toBeVisible();
    await expect(panel.getByText("Video on this page")).toHaveCount(0);
    await expect(panel.getByRole("button", { name: "Download video from this page" })).toHaveCount(
      0,
    );

    // Asked directly, as the popup would, the background refuses and sends
    // nothing.
    const result = await panel.evaluate(
      (id) =>
        chrome.runtime.sendMessage({
          type: "boltget:media-send",
          tabId: id,
          hint: null,
          sources: [],
        }),
      tabId,
    );
    expect(result).toBe("excluded");
    const state = await panel.evaluate(
      (id) => chrome.runtime.sendMessage({ type: "boltget:media-state", tabId: id }),
      tabId,
    );
    expect(state).toEqual({ allowed: false, hints: [] });
    noMediaPage(stub);
  });
}

test("the store policy: a YouTube player embedded elsewhere gives no hints", async ({
  context,
  worker,
  stub,
}) => {
  // A blog embeds a player from youtube-nocookie.com, which loads its stream.
  await context.route("https://blog.example/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "text/html",
      body: `<!doctype html><title>Blog</title>
        <iframe src="https://www.youtube-nocookie.com/embed/abc"></iframe>`,
    }),
  );
  await serve(context, "https://www.youtube-nocookie.com", "https://cdn.video.example");
  const page = await context.newPage();
  await page.goto("https://blog.example/post");
  await expect(page.frameLocator("iframe").locator("video")).toHaveCount(1);
  const frame = page.frames().find((f) => f.url().includes("youtube-nocookie"));
  await expect.poll(() => frame?.title()).toBe("A clip (loaded)");
  const tabId = await tabOf(worker, "https://blog.example/");
  await page.waitForTimeout(500);
  expect((await sessionMedia(worker, tabId))?.hints ?? []).toEqual([]);
  expect(await badge(worker, tabId)).toBe("");
  noMediaPage(stub);
});
