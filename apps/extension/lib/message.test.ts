import { describe, expect, it } from "vitest";
import {
  addLinksMessage,
  captureMessage,
  cleanFilename,
  cleanLinks,
  cleanMime,
  cleanText,
  clip,
  MAX_LINKS,
  MAX_TEXT,
  mediaPageMessage,
} from "./message";

describe("captureMessage", () => {
  const base = {
    url: "https://e.com/get?id=1",
    incognito: false,
    forced: false,
    cookies: [],
    headers: [] as [string, string][],
  };

  it("sends what is known, in the protocol's names", () => {
    expect(
      captureMessage({
        ...base,
        finalUrl: "https://cdn.e.com/a.zip",
        filename: "C:\\Users\\me\\Downloads\\a.zip",
        size: 1234,
        mime: "application/zip",
        referrer: "https://e.com/page",
        pageUrl: "https://e.com/page",
        headers: [["user-agent", "UA"]],
      }),
    ).toEqual({
      url: "https://e.com/get?id=1",
      final_url: "https://cdn.e.com/a.zip",
      filename: "a.zip",
      size: 1234,
      mime: "application/zip",
      referrer: "https://e.com/page",
      page_url: "https://e.com/page",
      incognito: false,
      forced: false,
      cookies: [],
      headers: [["user-agent", "UA"]],
    });
  });

  it("leaves out what the host would refuse instead of sending it", () => {
    const m = captureMessage({
      ...base,
      finalUrl: "https://e.com/get?id=1",
      filename: "",
      size: -1,
      mime: "x".repeat(300),
      referrer: "chrome://newtab/",
      pageUrl: "file:///C:/page.html",
    });
    expect(m).toEqual({ ...base, url: base.url });
  });

  it("falls back to the page for the referrer", () => {
    expect(captureMessage({ ...base, pageUrl: "https://e.com/p" }).referrer).toBe(
      "https://e.com/p",
    );
  });

  it("keeps sizes the protocol can carry", () => {
    expect(captureMessage({ ...base, size: 2 ** 40 }).size).toBe(2 ** 40);
    expect(captureMessage({ ...base, size: 1.5 }).size).toBeUndefined();
  });
});

describe("cleaning", () => {
  it("file names: no folders, no control characters, at most 1 KiB", () => {
    expect(cleanFilename("/home/me/a.zip")).toBe("a.zip");
    expect(cleanFilename("a\u0000b\u001f.zip")).toBe("ab.zip");
    expect(cleanFilename("..")).toBe(null);
    expect(cleanFilename(null)).toBe(null);
    expect(
      new TextEncoder().encode(cleanFilename("é".repeat(1000)) ?? "").length,
    ).toBeLessThanOrEqual(MAX_TEXT);
  });

  it("clips by UTF-8 bytes without splitting a character", () => {
    expect(clip("aé", 2)).toBe("a");
    expect(clip("abc", 3)).toBe("abc");
    expect(clip("😀😀", 5)).toBe("😀");
  });

  it("MIME types: printable and short", () => {
    expect(cleanMime(" video/mp4 ")).toBe("video/mp4");
    expect(cleanMime("vidéo/mp4")).toBe(null);
    expect(cleanMime("")).toBe(null);
  });

  it("texts: one line", () => {
    expect(cleanText("  Get\n the\tfile  ")).toBe("Get the file");
    expect(cleanText("   ")).toBe(null);
  });
});

describe("links", () => {
  it("keeps http(s) links once each, fragments dropped, texts cleaned", () => {
    expect(
      cleanLinks([
        { url: "https://e.com/a.zip", text: " A " },
        { url: "https://e.com/a.zip#top", text: "again" },
        { url: "javascript:void(0)" },
        { url: "mailto:me@e.com" },
        { url: "blob:https://e.com/1" },
        { url: "http://e.com/b.zip", text: "" },
      ]),
    ).toEqual([{ url: "https://e.com/a.zip", text: "A" }, { url: "http://e.com/b.zip" }]);
  });

  it("stops at 5000", () => {
    const many = Array.from({ length: MAX_LINKS + 10 }, (_, i) => ({ url: `https://e.com/${i}` }));
    expect(cleanLinks(many)).toHaveLength(MAX_LINKS);
  });

  it("add_links needs at least one link", () => {
    expect(addLinksMessage([{ url: "javascript:x" }], "https://e.com/", false, [], [])).toBeNull();
    expect(addLinksMessage([{ url: "https://e.com/a" }], "https://e.com/", true, [], [])).toEqual({
      links: [{ url: "https://e.com/a" }],
      page_url: "https://e.com/",
      incognito: true,
      cookies: [],
      headers: [],
    });
  });

  it("media_page needs a page the app can fetch", () => {
    expect(mediaPageMessage("chrome://newtab", "x", false, [])).toBeNull();
    expect(mediaPageMessage("https://e.com/v", " Video ", false, [])).toEqual({
      page_url: "https://e.com/v",
      title: "Video",
      incognito: false,
      cookies: [],
      headers: [],
    });
  });
});
