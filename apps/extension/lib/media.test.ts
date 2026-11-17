import { MAX_HINTS } from "@boltget/native-messaging";
import { describe, expect, it } from "vitest";
import {
  addHint,
  badgeText,
  classifyResponse,
  hintLine,
  hintsToSend,
  linkResolution,
  MAX_TAB_HINTS,
  MIN_MEDIA_SIZE,
  pageChanged,
  pageKey,
  pathExtension,
  readMediaRequest,
  readScan,
  responseSize,
  sizeText,
  sourceHint,
  TabMediaStore,
  type Hint,
  type ResponseInfo,
} from "./media";

const MIB = 1024 * 1024;

function response(
  url: string,
  type: string | null,
  extra: Record<string, string> = {},
  over: Partial<ResponseInfo> = {},
): ResponseInfo {
  const headers = Object.entries({ ...(type === null ? {} : { "Content-Type": type }), ...extra });
  return {
    url,
    statusCode: 200,
    method: "GET",
    responseHeaders: headers.map(([name, value]) => ({ name, value })),
    ...over,
  };
}

describe("classifyResponse", () => {
  it("finds HLS and DASH manifests by type, or by path when the type says nothing", () => {
    for (const type of [
      "application/vnd.apple.mpegurl",
      "application/x-mpegURL; charset=utf-8",
      "audio/mpegurl",
      "audio/x-mpegurl",
    ]) {
      expect(classifyResponse(response("https://cdn.test/live", type))?.kind, type).toBe("hls");
    }
    expect(classifyResponse(response("https://cdn.test/a/master.m3u8?t=1", "text/plain"))).toEqual({
      url: "https://cdn.test/a/master.m3u8?t=1",
      kind: "hls",
      mime: "text/plain",
    });
    expect(classifyResponse(response("https://cdn.test/a.m3u8", null))?.kind).toBe("hls");
    expect(classifyResponse(response("https://cdn.test/m", "application/dash+xml"))?.kind).toBe(
      "dash",
    );
    expect(
      classifyResponse(response("https://cdn.test/a.MPD", "application/octet-stream"))?.kind,
    ).toBe("dash");
    // A path alone does not overrule a type that says something else.
    expect(classifyResponse(response("https://cdn.test/a.m3u8", "text/html"))).toBeNull();
  });

  it("finds video and audio files, with their size", () => {
    const mp4 = classifyResponse(
      response("https://cdn.test/v.mp4", "video/mp4", { "Content-Length": String(40 * MIB) }),
    );
    expect(mp4).toEqual({
      url: "https://cdn.test/v.mp4",
      kind: "file",
      mime: "video/mp4",
      size: 40 * MIB,
    });
    expect(classifyResponse(response("https://cdn.test/song", "audio/mpeg"))?.kind).toBe("file");
    // A generic type with a media extension.
    expect(
      classifyResponse(response("https://cdn.test/v.webm", "application/octet-stream"))?.kind,
    ).toBe("file");
    expect(
      classifyResponse(response("https://cdn.test/v.zip", "application/octet-stream")),
    ).toBeNull();
    // A player's range request: the total size, from its first request only.
    const ranged = classifyResponse(
      response(
        "https://cdn.test/v.mp4",
        "video/mp4",
        { "Content-Range": `bytes 0-1023/${90 * MIB}`, "Content-Length": "1024" },
        { statusCode: 206 },
      ),
    );
    expect(ranged?.size).toBe(90 * MIB);
    expect(
      classifyResponse(
        response(
          "https://cdn.test/v.mp4",
          "video/mp4",
          { "Content-Range": `bytes 5000-9999/${90 * MIB}` },
          { statusCode: 206 },
        ),
      ),
    ).toBeNull();
    // Size unknown: kept.
    expect(classifyResponse(response("https://cdn.test/live.webm", "video/webm"))?.size).toBe(
      undefined,
    );
  });

  it("leaves out segments, small files, failures and what is not media", () => {
    const none = [
      response("https://cdn.test/seg1.ts", "video/mp2t", { "Content-Length": String(2 * MIB) }),
      response("https://cdn.test/seg1.ts", "application/octet-stream"),
      response("https://cdn.test/c/seg-5.m4s", "video/iso.segment"),
      response("https://cdn.test/c/seg-5.m4s", "video/mp4", { "Content-Length": String(2 * MIB) }),
      response("https://cdn.test/a.aac", "audio/aac"),
      response("https://cdn.test/click.mp4", "video/mp4", {
        "Content-Length": String(MIN_MEDIA_SIZE - 1),
      }),
      response("https://cdn.test/page", "text/html"),
      response("https://cdn.test/img.png", "image/png"),
      response("https://cdn.test/sub.vtt", "text/vtt"),
      response("https://cdn.test/v.mp4", "video/mp4", {}, { statusCode: 404 }),
      response("https://cdn.test/v.mp4", "video/mp4", {}, { statusCode: 304 }),
      response("https://cdn.test/v.mp4", "video/mp4", {}, { method: "POST" }),
      response("blob:https://cdn.test/0f", "video/mp4"),
      response("data:video/mp4;base64,AAAA", "video/mp4"),
    ];
    for (const r of none) expect(classifyResponse(r), r.url).toBeNull();
    expect(
      classifyResponse(
        response("https://cdn.test/v.mp4", "video/mp4", {
          "Content-Length": String(MIN_MEDIA_SIZE),
        }),
      )?.kind,
    ).toBe("file");
  });

  it("reads sizes and extensions defensively", () => {
    expect(responseSize([{ name: "content-length", value: "12" }])).toBe(12);
    expect(responseSize([{ name: "Content-Range", value: "bytes 0-1/*" }])).toBeNull();
    expect(responseSize([{ name: "content-length", value: "-1" }])).toBeNull();
    expect(responseSize([{ name: "content-length", value: "99999999999999999999" }])).toBeNull();
    expect(responseSize(undefined)).toBeNull();
    expect(pathExtension("https://a.test/x/v.MP4?y=.m3u8")).toBe("mp4");
    expect(pathExtension("https://a.test/.hidden")).toBe("");
    expect(pathExtension("nonsense")).toBe("");
  });
});

describe("a tab's hints", () => {
  const file = (n: number): Hint => ({ url: `https://cdn.test/${n}.mp4`, kind: "file" });
  const hls: Hint = { url: "https://cdn.test/master.m3u8", kind: "hls" };

  it("keeps each link once and caps the list", () => {
    let list: Hint[] = [];
    for (let i = 0; i < MAX_TAB_HINTS + 5; i++) list = addHint(list, file(i)) ?? list;
    expect(list).toHaveLength(MAX_TAB_HINTS);
    expect(MAX_TAB_HINTS).toBe(MAX_HINTS);
    expect(addHint([file(1)], file(1))).toBeNull();
  });

  it("groups files under a manifest: they are its segments", () => {
    const withFiles = [file(1), file(2)];
    expect(addHint(withFiles, hls)).toEqual([hls]);
    expect(addHint([hls], file(3))).toBeNull();
    const dash: Hint = { url: "https://cdn.test/a.mpd", kind: "dash" };
    expect(addHint([hls], dash)).toEqual([hls, dash]);
  });

  it("is cleared when the tab moves to another page, not for a fragment", () => {
    expect(pageKey("https://a.test/v#t=10")).toBe("https://a.test/v");
    expect(pageChanged("https://a.test/v", "https://a.test/v#t=10")).toBe(false);
    expect(pageChanged("https://a.test/v", "https://a.test/w")).toBe(true);
    expect(pageChanged(null, "https://a.test/w")).toBe(true);
    expect(pageChanged("https://a.test/v", undefined)).toBe(false);
  });

  it("shows a count on the badge", () => {
    expect(badgeText(0)).toBe("");
    expect(badgeText(3)).toBe("3");
    expect(badgeText(10)).toBe("9+");
  });
});

describe("what goes with media_page", () => {
  const hls: Hint = { url: "https://cdn.test/a.m3u8", kind: "hls", mime: "application/x-mpegurl" };

  it("sends the chosen hint alone, or everything once", () => {
    expect(hintsToSend([hls], ["https://cdn.test/v.mp4"], hls.url)).toEqual([hls]);
    expect(hintsToSend([], [], "https://cdn.test/x.mpd")).toEqual([
      { url: "https://cdn.test/x.mpd", kind: "dash" },
    ]);
    expect(hintsToSend([hls], ["https://cdn.test/a.m3u8", "https://cdn.test/v.mp4"], null)).toEqual(
      [hls, { url: "https://cdn.test/v.mp4", kind: "file" }],
    );
    const many = Array.from({ length: 30 }, (_, i) => `https://cdn.test/${i}.mp4`);
    expect(hintsToSend([], many, null)).toHaveLength(MAX_TAB_HINTS);
    expect(sourceHint("blob:https://a.test/1")).toBeNull();
  });

  it("takes from a page scan only what has the right shape", () => {
    expect(readScan(null)).toEqual({ elements: 0, declared: false, sources: [] });
    expect(
      readScan({ elements: 2, declared: true, sources: ["https://a.test/v.mp4", "blob:x", 3] }),
    ).toEqual({
      elements: 2,
      declared: true,
      sources: ["https://a.test/v.mp4"],
    });
    expect(readScan({ elements: -4, declared: "yes" })).toEqual({
      elements: 0,
      declared: false,
      sources: [],
    });
  });

  it("names a resolution only where the link has one", () => {
    expect(linkResolution("https://a.test/hls/720p/index.m3u8")).toBe("720p");
    expect(linkResolution("https://a.test/v_1920x1080.mp4")).toBe("1080p");
    expect(linkResolution("https://a.test/v/1234567.mp4")).toBeNull();
  });
});

describe("TabMediaStore", () => {
  function area() {
    const data = new Map<string, unknown>();
    return {
      data,
      get: (key: string) => Promise.resolve(data.has(key) ? { [key]: data.get(key) } : {}),
      set: (items: Record<string, unknown>) => {
        for (const [k, v] of Object.entries(items)) data.set(k, structuredClone(v));
        return Promise.resolve();
      },
      remove: (key: string) => {
        data.delete(key);
        return Promise.resolve();
      },
    };
  }

  it("keeps each tab's hints in order, reports counts, and clears them on navigation", async () => {
    const a = area();
    const counts: [number, number][] = [];
    const store = new TabMediaStore(a, (tab, n) => counts.push([tab, n]));
    await store.navigated(7, "https://v.test/watch?v=1");
    // Added at once, in the order seen, without lost updates.
    await Promise.all([
      store.add(7, { url: "https://cdn.test/1.mp4", kind: "file" }),
      store.add(7, { url: "https://cdn.test/2.mp4", kind: "file" }),
      store.add(7, { url: "https://cdn.test/1.mp4", kind: "file" }),
      store.add(8, { url: "https://cdn.test/a.m3u8", kind: "hls" }),
    ]);
    expect((await store.get(7)).hints.map((h) => h.url)).toEqual([
      "https://cdn.test/1.mp4",
      "https://cdn.test/2.mp4",
    ]);
    expect((await store.get(7)).page).toBe("https://v.test/watch?v=1");
    expect(counts).toEqual([
      [7, 1],
      [7, 2],
      [8, 1],
    ]);
    // A fragment is the same page; another page clears the tab.
    await store.navigated(7, "https://v.test/watch?v=1#t=3");
    expect((await store.get(7)).hints).toHaveLength(2);
    await store.navigated(7, "https://v.test/watch?v=2");
    expect(await store.get(7)).toEqual({ page: "https://v.test/watch?v=2", hints: [] });
    expect(counts.at(-1)).toEqual([7, 0]);
    await store.forget(8);
    expect(a.data.has("media:8")).toBe(false);
    expect(await store.get(8)).toEqual({ page: null, hints: [] });
  });

  it("reads only well-formed records back", async () => {
    const a = area();
    a.data.set("media:1", { page: 3, hints: [{ url: "javascript:x", kind: "file" }, "x", null] });
    a.data.set("media:2", "nonsense");
    const store = new TabMediaStore(a, () => {});
    expect(await store.get(1)).toEqual({ page: null, hints: [] });
    expect(await store.get(2)).toEqual({ page: null, hints: [] });
  });
});

describe("requests from the popup", () => {
  it("are read only in their own shapes", () => {
    expect(readMediaRequest({ type: "boltget:media-state", tabId: 4 })).toEqual({
      type: "boltget:media-state",
      tabId: 4,
    });
    expect(
      readMediaRequest({
        type: "boltget:media-send",
        tabId: 4,
        hint: "https://cdn.test/a.m3u8",
        sources: ["https://cdn.test/v.mp4", "javascript:alert(1)"],
        extra: true,
      }),
    ).toEqual({
      type: "boltget:media-send",
      tabId: 4,
      hint: "https://cdn.test/a.m3u8",
      sources: ["https://cdn.test/v.mp4"],
    });
    expect(
      readMediaRequest({ type: "boltget:media-send", tabId: 1, hint: "file:///C:/x", sources: 3 }),
    ).toEqual({ type: "boltget:media-send", tabId: 1, hint: null, sources: [] });
    for (const bad of [
      null,
      "boltget:media-state",
      { type: "boltget:media-state" },
      { type: "boltget:media-state", tabId: -1 },
      { type: "boltget:media-state", tabId: 1.5 },
      { type: "other", tabId: 1 },
      { href: "https://a.test/", alt: true, shift: false },
    ]) {
      expect(readMediaRequest(bad)).toBeNull();
    }
  });
});

describe("hints in the popup", () => {
  it("say what they are, then resolution, host and size", () => {
    expect(
      hintLine({ url: "https://cdn.test/720p/a.m3u8", kind: "hls", mime: "application/x-mpegurl" }),
    ).toEqual({ kind: "media.kind.hls", host: "cdn.test", details: ["720p", "cdn.test"] });
    expect(hintLine({ url: "https://cdn.test/a.mpd", kind: "dash" }).kind).toBe("media.kind.dash");
    expect(
      hintLine({ url: "https://cdn.test/a.m4a", kind: "file", mime: "audio/mp4", size: 5 * MIB }),
    ).toEqual({ kind: "media.kind.audio", host: "cdn.test", details: ["cdn.test", "5 MiB"] });
    expect(hintLine({ url: "https://cdn.test/a.mp4", kind: "file" }).kind).toBe("media.kind.video");
  });

  it("show sizes in binary units", () => {
    expect(sizeText(512)).toBe("512 B");
    expect(sizeText(512 * 1024)).toBe("512 KiB");
    expect(sizeText(48.25 * MIB)).toBe("48.3 MiB");
    expect(sizeText(1.5 * 1024 * MIB)).toBe("1.5 GiB");
  });
});
