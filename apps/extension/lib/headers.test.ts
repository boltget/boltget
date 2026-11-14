import { describe, expect, it } from "vitest";
import { filterHeaders, HeaderLog, MAX_HEADER_VALUE } from "./headers";

describe("filterHeaders", () => {
  it("keeps only the allowlist, lowercased, each name once", () => {
    expect(
      filterHeaders([
        { name: "User-Agent", value: "UA" },
        { name: "Cookie", value: "secret=1" },
        { name: "Referer", value: "https://e.com/" },
        { name: "referer", value: "https://other.com/" },
        { name: "Accept", value: "*/*" },
        { name: "Accept-Language", value: "en" },
        { name: "Origin", value: "https://e.com" },
        { name: "Authorization", value: "Bearer t" },
        { name: "Sec-Fetch-Mode", value: "navigate" },
        { name: "Range", value: "bytes=0-" },
      ]),
    ).toEqual([
      ["user-agent", "UA"],
      ["referer", "https://e.com/"],
      ["accept", "*/*"],
      ["accept-language", "en"],
      ["origin", "https://e.com"],
      ["authorization", "Bearer t"],
    ]);
  });

  it("drops values the host would refuse", () => {
    expect(
      filterHeaders([
        { name: "Referer", value: "a\r\nInjected: 1" },
        { name: "Accept", value: "x".repeat(MAX_HEADER_VALUE + 1) },
        { name: "Origin" },
        { name: "User-Agent", value: "" },
      ]),
    ).toEqual([]);
    expect(filterHeaders(undefined)).toEqual([]);
  });
});

describe("HeaderLog", () => {
  it("gives the request for the final URL first, then the link, once", () => {
    const log = new HeaderLog();
    log.record("https://e.com/get", "GET", [{ name: "Referer", value: "r1" }], 0);
    log.record("https://cdn.e.com/a.zip", "GET", [{ name: "Referer", value: "r2" }], 1);
    expect(log.take(["https://cdn.e.com/a.zip", "https://e.com/get"], 2)?.headers).toEqual([
      ["referer", "r2"],
    ]);
    expect(log.take(["https://cdn.e.com/a.zip"], 3)).toBeNull();
    expect(log.take([null, "https://e.com/get"], 3)?.method).toBe("GET");
  });

  it("forgets requests after 30 s", () => {
    const log = new HeaderLog();
    log.record("https://e.com/a", "GET", [], 0);
    expect(log.take(["https://e.com/a"], 30_001)).toBeNull();
    log.record("https://e.com/b", "POST", [], 0);
    expect(log.take(["https://e.com/b"], 30_000)?.method).toBe("POST");
  });

  it("keeps at most its cap, dropping the oldest", () => {
    const log = new HeaderLog(30_000, 3);
    for (let i = 0; i < 5; i++) log.record(`https://e.com/${i}`, "GET", [], i);
    expect(log.size).toBe(3);
    expect(log.take(["https://e.com/0"], 5)).toBeNull();
    expect(log.take(["https://e.com/4"], 5)).not.toBeNull();
  });

  it("a later request for a URL replaces the earlier one", () => {
    const log = new HeaderLog(30_000, 2);
    log.record("https://e.com/a", "GET", [], 0);
    log.record("https://e.com/b", "GET", [], 1);
    log.record("https://e.com/a", "POST", [], 2);
    log.record("https://e.com/c", "GET", [], 3);
    expect(log.take(["https://e.com/b"], 4)).toBeNull();
    expect(log.take(["https://e.com/a"], 4)?.method).toBe("POST");
  });
});
