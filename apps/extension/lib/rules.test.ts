import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  DEFAULT_EXTENSIONS,
  DEFAULT_RULES,
  hostMatches,
  hostOf,
  normaliseExtension,
  normaliseRules,
  normaliseSite,
  onAnySite,
  splitList,
} from "./rules";

describe("defaults", () => {
  it("are the app's (crates/ipc/src/browser.rs, DEFAULT_EXTENSIONS)", () => {
    const rust = readFileSync(
      new URL("../../../crates/ipc/src/browser.rs", import.meta.url),
      "utf8",
    );
    const match = /pub const DEFAULT_EXTENSIONS: &str = "([^"]*)";/s.exec(rust);
    expect(match).not.toBeNull();
    const list = (match?.[1] ?? "").replace(/\\\s*/g, " ").split(/\s+/).filter(Boolean);
    expect([...DEFAULT_EXTENSIONS]).toEqual(list);
    expect(rust).toContain("min_size: 1 << 20");
    expect(DEFAULT_RULES.min_size).toBe(1 << 20);
    expect(DEFAULT_RULES.private_windows).toBe(false);
  });

  it("do not capture documents or torrents", () => {
    for (const ext of ["pdf", "docx", "xlsx", "torrent", "txt"]) {
      expect(DEFAULT_RULES.extensions).not.toContain(ext);
    }
  });
});

describe("normalising", () => {
  it.each([
    ["ZIP", "zip"],
    [".7z", "7z"],
    ["*.iso", "iso"],
    ["  mp4 ", "mp4"],
    ["tar.gz", null],
    ["", null],
    ["a b", null],
    ["x".repeat(17), null],
  ])("extension %j -> %j", (raw, ext) => {
    expect(normaliseExtension(raw)).toBe(ext);
  });

  it.each([
    ["example.com", "example.com"],
    ["https://Example.com/path?q", "example.com"],
    ["*.example.com", "example.com"],
    [".example.com.", "example.com"],
    ["example.com:8443", "example.com"],
    ["http://user@example.com", null],
    ["", null],
    ["exa mple.com", null],
    ["a..b", null],
  ])("site %j -> %j", (raw, site) => {
    expect(normaliseSite(raw)).toBe(site);
  });

  it("splits lists on spaces, commas, semicolons and lines", () => {
    expect(splitList("zip, rar;7z\n iso\t\tmp4")).toEqual(["zip", "rar", "7z", "iso", "mp4"]);
    expect(splitList("   ")).toEqual([]);
  });

  it("makes whatever storage or the app gave safe", () => {
    expect(normaliseRules(undefined)).toEqual(DEFAULT_RULES);
    expect(normaliseRules("nonsense")).toEqual(DEFAULT_RULES);
    expect(
      normaliseRules({
        enabled: false,
        min_size: 5.7,
        extensions: ["ZIP", "zip", 3, ".rar", "bad ext"],
        never_sites: ["Bank.example", "bank.example", null],
        always_sites: "not a list",
        private_windows: "yes",
      }),
    ).toEqual({
      enabled: false,
      min_size: 5,
      extensions: ["zip", "rar"],
      never_sites: ["bank.example"],
      always_sites: [],
      private_windows: false,
    });
    expect(normaliseRules({ min_size: -1 }).min_size).toBe(DEFAULT_RULES.min_size);
    expect(normaliseRules({ min_size: Number.NaN }).min_size).toBe(DEFAULT_RULES.min_size);
    expect(normaliseRules({ extensions: [] }).extensions).toEqual([]);
  });
});

describe("site matching", () => {
  it("matches the host and its subdomains on label boundaries", () => {
    expect(hostMatches("example.com", "example.com")).toBe(true);
    expect(hostMatches("a.b.example.com", "example.com")).toBe(true);
    expect(hostMatches("EXAMPLE.COM.", "example.com")).toBe(true);
    expect(hostMatches("badexample.com", "example.com")).toBe(false);
    expect(hostMatches("example.com.evil", "example.com")).toBe(false);
  });

  it("reads hosts from URLs", () => {
    expect(hostOf("https://Sub.Example.com:8080/x")).toBe("sub.example.com");
    expect(hostOf("data:text/plain,hi")).toBe(null);
    expect(hostOf("nonsense")).toBe(null);
    expect(hostOf(null)).toBe(null);
  });

  it("checks any of several URLs", () => {
    expect(onAnySite([null, "https://x.e.com/"], ["e.com"])).toBe(true);
    expect(onAnySite(["https://x.e.com/"], [])).toBe(false);
    expect(onAnySite([undefined, "junk"], ["e.com"])).toBe(false);
  });
});
