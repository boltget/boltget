import { describe, expect, it } from "vitest";
import { hintAllowed, mediaAllowed, MEDIA_EXCLUDED_SITES } from "./policy";

describe("the store policy", () => {
  it("excludes YouTube's sites and every subdomain", () => {
    for (const url of [
      "https://youtube.com/watch?v=1",
      "https://www.youtube.com/watch?v=1",
      "https://m.youtube.com/watch?v=1",
      "https://music.youtube.com/watch?v=1",
      "https://studio.youtube.com/",
      "https://WWW.YouTube.COM/shorts/x",
      "https://www.youtube.com./watch?v=1",
      "https://www.youtube.com:443/watch?v=1",
      "http://youtube.com/",
      "https://youtu.be/abc",
      "https://www.youtube-nocookie.com/embed/abc",
      "https://youtube-nocookie.com/embed/abc",
      "https://user:pw@www.youtube.com/",
    ]) {
      expect(mediaAllowed(url, true), url).toBe(false);
      // Without the policy (a build no store sees), they are like any site.
      expect(mediaAllowed(url, false), url).toBe(true);
    }
    expect(MEDIA_EXCLUDED_SITES).toContain("youtube.com");
  });

  it("does not exclude look-alikes", () => {
    for (const url of [
      "https://notyoutube.com/watch",
      "https://myyoutube.com/",
      "https://youtube.com.evil.test/watch",
      "https://youtube.co/",
      "https://youtube.de.example/",
      "https://youtu.be.example.org/",
      "https://ytube.com/",
      "https://example.com/youtube.com/watch",
      "https://example.com/?u=https://www.youtube.com/",
      "https://youtube-nocookie.com.example/",
      "https://notyoutu.be/",
      "https://vimeo.com/123",
    ]) {
      expect(mediaAllowed(url, true), url).toBe(true);
    }
  });

  it("offers media only on http and https pages", () => {
    for (const url of [
      null,
      undefined,
      "",
      "chrome://newtab",
      "edge://settings",
      "about:blank",
      "file:///C:/v.mp4",
      "chrome-extension://abc/popup.html",
      "view-source:https://example.com/",
      "not a url",
    ]) {
      expect(mediaAllowed(url, false), String(url)).toBe(false);
    }
  });

  it("drops hints from YouTube's hosts and from what they load, wherever the page is", () => {
    const page = "https://blog.example/post";
    expect(hintAllowed("https://cdn.example/v.m3u8", [page], true)).toBe(true);
    // YouTube's media servers, and a player embedded from an excluded site.
    expect(hintAllowed("https://rr3---sn-x.googlevideo.com/videoplayback?x", [page], true)).toBe(
      false,
    );
    expect(
      hintAllowed("https://cdn.example/v.m3u8", ["https://www.youtube-nocookie.com"], true),
    ).toBe(false);
    expect(hintAllowed("https://cdn.example/v.m3u8", [null, "https://www.youtube.com"], true)).toBe(
      false,
    );
    expect(hintAllowed("https://notgooglevideo.com/v.mp4", [page], true)).toBe(true);
    // Without the policy nothing is dropped.
    expect(hintAllowed("https://rr3---sn-x.googlevideo.com/videoplayback", [], false)).toBe(true);
  });
});
