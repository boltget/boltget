import { describe, expect, it, vi } from "vitest";
import { DEFAULT_SETTINGS } from "../settings/model";
import { offerFor, simulateCopy, VIDEO_SITES, videoPage, watchClipboard } from "./mock-clipboard";

// The browser-only UI must offer what the app offers: the same list, and
// the same examples the app's tests check (src-tauri/src/clipboard/video.rs).
describe("video page links in the browser-only UI", () => {
  it("offers every example with its site", () => {
    for (const site of VIDEO_SITES.sites) {
      for (const example of site.examples) {
        expect(videoPage(example), example).toEqual({
          site: site.site,
          audio: site.audio ?? false,
        });
      }
    }
  });

  it("offers none of the lookalikes", () => {
    for (const link of VIDEO_SITES.notPages) expect(videoPage(link), link).toBeNull();
  });

  it("prefers a page to a file, only when the setting is on", () => {
    const on = { ...DEFAULT_SETTINGS, clipboardVideo: true };
    const off = { ...DEFAULT_SETTINGS, clipboardVideo: false };
    expect(offerFor("https://youtu.be/abc\n", on)).toEqual({
      kind: "page",
      url: "https://youtu.be/abc",
      site: "YouTube",
      audio: false,
    });
    expect(offerFor("https://youtu.be/abc", off)).toBeNull();
    const commons = "https://commons.wikimedia.org/wiki/File:Big_Buck_Bunny_4K.webm";
    expect(offerFor(commons, on)?.kind).toBe("page");
    expect(offerFor(commons, off)?.kind).toBe("files");
    expect(offerFor("https://x.test/a.zip https://x.test/page", on)).toEqual({
      kind: "files",
      links: "https://x.test/a.zip",
    });
    expect(offerFor("see https://youtu.be/abc", on)).toBeNull();
  });

  it("offers a link once, and only while monitoring is on", () => {
    let settings = { ...DEFAULT_SETTINGS, clipboard: false };
    const handler = vi.fn();
    const stop = watchClipboard(() => settings, handler);
    simulateCopy("https://youtu.be/abc");
    expect(handler).not.toHaveBeenCalled();
    settings = { ...settings, clipboard: true };
    simulateCopy("https://youtu.be/abc");
    simulateCopy("https://youtu.be/abc");
    expect(handler).toHaveBeenCalledTimes(1);
    stop();
    simulateCopy("https://youtu.be/def");
    expect(handler).toHaveBeenCalledTimes(1);
  });
});
