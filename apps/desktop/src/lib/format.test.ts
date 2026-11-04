import { describe, expect, it } from "vitest";
import {
  bytesParts,
  displayName,
  formatBytes,
  formatDuration,
  formatEta,
  formatProgress,
  formatSpeed,
  formatWhen,
  hostOf,
  pathPieces,
  percent,
  progressParts,
  secondsLeft,
  speedParts,
  splitFolder,
  stableEta,
  unitOf,
} from "./format";

const KIB = 1024;
const MIB = KIB * KIB;
const GIB = MIB * KIB;

describe("sizes", () => {
  it("uses binary units and rounds by magnitude", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.5 KiB");
    expect(formatBytes(48.24 * MIB)).toBe("48.2 MiB");
    expect(formatBytes(420 * MIB)).toBe("420 MiB");
    expect(formatBytes(1.25 * GIB)).toBe("1.25 GiB");
    expect(formatBytes(7 * MIB)).toBe("7 MiB");
  });

  it("moves to the next unit at 1000, so numbers stay three digits wide", () => {
    expect(unitOf(999)).toBe(0);
    expect(unitOf(1000)).toBe(1);
    expect(formatBytes(1000 * MIB)).toBe("0.98 GiB");
  });

  it("shows progress in the total's unit, with fixed decimals while it moves", () => {
    expect(formatProgress(212 * MIB, 420 * MIB)).toBe("212 / 420 MiB");
    expect(formatProgress(0.4 * GIB, 5.97 * GIB)).toBe("0.40 / 5.97 GiB");
    expect(formatProgress(512 * MIB, null)).toBe("512 MiB");
    expect(formatProgress(600, 500)).toBe("500 / 500 B");
  });

  it("formats speeds per second", () => {
    expect(formatSpeed(48.2 * MIB)).toBe("48.2 MiB/s");
    expect(formatSpeed(5 * MIB)).toBe("5.00 MiB/s");
    expect(formatSpeed(0)).toBe("0 B/s");
  });

  it("cuts messages around their number for the large stats", () => {
    expect(bytesParts(420 * MIB)).toEqual({ before: "", value: "420", after: " MiB" });
    expect(speedParts(48.2 * MIB)).toEqual({ before: "", value: "48.2", after: " MiB/s" });
    expect(progressParts(212 * MIB, 420 * MIB)).toEqual({
      before: "",
      value: "212",
      after: " / 420 MiB",
    });
  });
});

describe("time left", () => {
  it("is precise when short and in words when long", () => {
    expect(formatEta(0.2)).toBe("1 s left");
    expect(formatEta(12)).toBe("12 s left");
    expect(formatEta(170)).toBe("about 3 min left");
    expect(formatEta(3600)).toBe("about 1 h left");
    expect(formatEta(7800)).toBe("about 2 h 10 min left");
    expect(formatEta(11 * 3600 + 600)).toBe("about 11 h left");
    expect(formatEta(2 * 86_400)).toBe("more than a day left");
  });

  it("is unknown without a total or a speed", () => {
    expect(secondsLeft(10, null, 100)).toBeNull();
    expect(secondsLeft(10, 100, 0)).toBeNull();
    expect(secondsLeft(10, 110, 10)).toBe(10);
  });

  it("changes only when the estimate moves by more than 10%", () => {
    expect(stableEta(null, 100)).toBe(100);
    expect(stableEta(100, 95)).toBe(100);
    expect(stableEta(100, 109)).toBe(100);
    expect(stableEta(100, 89)).toBe(89);
    expect(stableEta(100, null)).toBeNull();
  });

  it("formats durations for the stats", () => {
    expect(formatDuration(8)).toBe("8 s");
    expect(formatDuration(240)).toBe("4 min");
    expect(formatDuration(7800)).toBe("2 h 10 min");
  });
});

describe("names and places", () => {
  it("prefers the probed name, then the link's last segment, then the host", () => {
    expect(displayName("a.iso", "https://x.org/b.iso")).toBe("a.iso");
    expect(displayName(null, "https://x.org/dir/My%20File.zip?x=1")).toBe("My File.zip");
    expect(displayName(null, "https://x.org/")).toBe("x.org");
    expect(displayName(null, "not a url")).toBe("not a url");
  });

  it("shows hosts without www", () => {
    expect(hostOf("https://www.example.com/a")).toBe("example.com");
    expect(hostOf("https://releases.ubuntu.com/a")).toBe("releases.ubuntu.com");
  });

  it("shows today's times and other days' dates", () => {
    const now = new Date(2026, 10, 1, 15, 0).getTime();
    expect(formatWhen(new Date(2026, 10, 1, 9, 5).getTime(), now)).toMatch(/9:05/);
    expect(formatWhen(new Date(2026, 9, 30, 9, 5).getTime(), now)).toBe("Oct 30");
    expect(formatWhen(new Date(2025, 9, 30).getTime(), now)).toBe("Oct 30, 2025");
  });

  it("never shows 100% before the end", () => {
    expect(percent(999, 1000)).toBe(99);
    expect(percent(1000, 1000)).toBe(100);
    expect(percent(0, 1000)).toBe(0);
    expect(percent(5, null)).toBeNull();
  });
});

describe("paths", () => {
  it("breaks only after separators", () => {
    const path = String.raw`C:\Users\you\scratch\1G.bin`;
    expect(pathPieces(path)).toEqual(["C:\\", "Users\\", "you\\", "scratch\\", "1G.bin"]);
    expect(pathPieces("https://a.org/b/c.dat")).toEqual(["https://a.", "org/", "b/", "c.dat"]);
    expect(pathPieces("plain").join("")).toBe("plain");
  });

  it("splits a folder into its name and parent", () => {
    expect(splitFolder(String.raw`C:\Users\you\Downloads`)).toEqual({
      parent: "C:\\Users\\you\\",
      leaf: "Downloads",
    });
    expect(splitFolder("D:\\")).toEqual({ parent: "", leaf: "D:" });
    expect(splitFolder("/home/me/")).toEqual({ parent: "/home/", leaf: "me" });
  });
});
