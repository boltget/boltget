import { describe, expect, it } from "vitest";
import {
  findDuplicate,
  linkKey,
  linksIn,
  parseChecksum,
  parseConnections,
  parseFileName,
  parseLimit,
  readLinks,
  type Listed,
} from "./links";

const MIB = 1024 * 1024;

describe("links in a text", () => {
  it("finds http and https links, once each, in order", () => {
    expect(
      linksIn("get <https://a.org/x.iso> and http://b.org/y.zip, also https://a.org/x.iso"),
    ).toEqual(["https://a.org/x.iso", "http://b.org/y.zip"]);
    expect(linksIn("ftp://c.org/z mailto:me@x.org plain words")).toEqual([]);
  });

  it("takes one link per line, as pasted from a list", () => {
    const text = "https://a.org/1.iso\r\n\n  https://a.org/2.iso  \nhttps://a.org/3.iso\n";
    expect(linksIn(text)).toEqual([
      "https://a.org/1.iso",
      "https://a.org/2.iso",
      "https://a.org/3.iso",
    ]);
  });

  it("strips the brackets and punctuation around a link", () => {
    expect(linksIn("(https://a.org/x.zip). [https://b.org/y]")).toEqual([
      "https://a.org/x.zip",
      "https://b.org/y",
    ]);
  });

  it("tells empty, not a link and links apart", () => {
    expect(readLinks("  \n")).toEqual({ kind: "empty" });
    expect(readLinks("cachefly.net/100mb")).toEqual({ kind: "invalid" });
    expect(readLinks("https://")).toEqual({ kind: "invalid" });
    expect(readLinks("https://a.org/x")).toEqual({ kind: "links", urls: ["https://a.org/x"] });
  });
});

describe("options", () => {
  it("connections: Auto when empty, 1 to 32 otherwise", () => {
    expect(parseConnections("")).toEqual({ ok: true, value: null });
    expect(parseConnections("Auto")).toEqual({ ok: true, value: null });
    expect(parseConnections(" 8 ")).toEqual({ ok: true, value: 8 });
    expect(parseConnections("32")).toEqual({ ok: true, value: 32 });
    for (const bad of ["0", "33", "2.5", "-1", "eight"]) {
      expect(parseConnections(bad).ok, bad).toBe(false);
    }
  });

  it("speed limit: MiB/s to bytes, empty for none", () => {
    expect(parseLimit("")).toEqual({ ok: true, value: null });
    expect(parseLimit("2")).toEqual({ ok: true, value: 2 * MIB });
    expect(parseLimit("0,5")).toEqual({ ok: true, value: MIB / 2 });
    expect(parseLimit(".25")).toEqual({ ok: true, value: MIB / 4 });
    for (const bad of ["0", "-1", "fast", "1.2.3"]) expect(parseLimit(bad).ok, bad).toBe(false);
  });

  it("checksum: what the engine's Checksum::parse accepts", () => {
    const sha256 = "a".repeat(64);
    expect(parseChecksum("")).toEqual({ ok: true, value: null });
    expect(parseChecksum(sha256)).toEqual({ ok: true, value: sha256 });
    expect(parseChecksum(`SHA-256: ${sha256}`)).toEqual({ ok: true, value: `sha256:${sha256}` });
    expect(parseChecksum(`md5=${"F".repeat(32)}`)).toEqual({
      ok: true,
      value: `md5:${"F".repeat(32)}`,
    });
    expect(parseChecksum(`sha:${"0".repeat(40)}`)).toEqual({
      ok: true,
      value: `sha1:${"0".repeat(40)}`,
    });
    for (const bad of [
      "a".repeat(63),
      `sha256:${"a".repeat(40)}`,
      `crc32:${"a".repeat(8)}`,
      "z".repeat(64),
    ]) {
      expect(parseChecksum(bad).ok, bad).toBe(false);
    }
  });

  it("file name: not empty, no folder", () => {
    expect(parseFileName(" a b (1).iso ")).toEqual({ ok: true, value: "a b (1).iso" });
    expect(parseFileName("")).toEqual({ ok: false, problem: "add.problem.name" });
    expect(parseFileName("..")).toEqual({ ok: false, problem: "add.problem.name" });
    for (const bad of ["a/b", "a\\b", "C:x"]) {
      expect(parseFileName(bad)).toEqual({ ok: false, problem: "add.problem.name_folder" });
    }
  });
});

describe("duplicates", () => {
  const items: Listed[] = [
    { id: 1, url: "https://a.org/x.iso", state: "completed", completedAt: 100 },
    { id: 2, url: "https://a.org/x.iso", state: "completed", completedAt: 300 },
    { id: 3, url: "https://a.org/y.iso?…", state: "paused", completedAt: null },
    { id: 4, url: "https://a.org/z.iso", state: "failed", completedAt: null },
  ];

  it("compares links as the list keeps them: no query or credentials", () => {
    expect(linkKey("https://user:pw@A.org/y.iso?sig=1#top")).toBe("https://a.org/y.iso");
    expect(linkKey("not a link")).toBeNull();
  });

  it("prefers the newest finished download", () => {
    expect(findDuplicate("https://a.org/x.iso", items)).toEqual({
      kind: "downloaded",
      id: 2,
      completedAt: 300,
    });
  });

  it("finds a row still in progress, even behind a signed query", () => {
    expect(findDuplicate("https://a.org/y.iso?token=abc", items)).toEqual({
      kind: "listed",
      id: 3,
    });
    expect(findDuplicate("https://a.org/z.iso", items)).toEqual({ kind: "listed", id: 4 });
  });

  it("is null for a new link", () => {
    expect(findDuplicate("https://a.org/new.iso", items)).toBeNull();
    expect(findDuplicate("http://a.org/x.iso", items)).toBeNull();
  });
});
