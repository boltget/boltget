import { t } from "@boltget/ui/messages";
import { describe, expect, it } from "vitest";
import {
  DEFAULT_CAPTURE,
  formatMinSize,
  groupState,
  MAX_RULE_EXTENSIONS,
  MAX_RULE_SITES,
  normaliseExtension,
  normaliseSite,
  otherTypes,
  parseMinSize,
  parseSites,
  sameCapture,
  setGroup,
  siteFromInput,
  TYPE_GROUPS,
  withOtherTypes,
} from "./capture";
import { DEFAULT_SETTINGS } from "./model";
import rust from "../../../../../crates/ipc/src/browser.rs?raw";
import rulesCases from "../../../../../crates/ipc/src/browser/rules-cases.json";
const group = (id: string) => TYPE_GROUPS.find((g) => g.id === id)!;

describe("capture defaults", () => {
  it("are the protocol's (crates/ipc/src/browser.rs)", () => {
    const match = /pub const DEFAULT_EXTENSIONS: &str = "([^"]*)";/s.exec(rust);
    expect(match).not.toBeNull();
    const list = (match?.[1] ?? "").replace(/\\\s*/g, " ").split(/\s+/).filter(Boolean);
    expect(DEFAULT_CAPTURE.extensions).toEqual(list);
    expect(rust).toContain("min_size: 1 << 20");
    expect(DEFAULT_CAPTURE.minSize).toBe(1 << 20);
    expect(rust).toContain(`MAX_RULE_EXTENSIONS: usize = ${MAX_RULE_EXTENSIONS};`);
    expect(rust).toContain(`MAX_RULE_SITES: usize = ${MAX_RULE_SITES};`);
    expect(DEFAULT_SETTINGS.capture).toEqual(DEFAULT_CAPTURE);
  });

  it("leave documents out, and no type is in two groups", () => {
    expect(groupState(DEFAULT_CAPTURE.extensions, group("documents"))).toBe("off");
    const all = TYPE_GROUPS.flatMap((g) => g.extensions);
    expect(new Set(all).size).toBe(all.length);
    for (const g of TYPE_GROUPS) {
      if (g.byDefault) expect(groupState(DEFAULT_CAPTURE.extensions, g)).toBe("on");
      for (const e of g.extensions) expect(normaliseExtension(e)).toBe(e);
    }
  });

  it("compare by value", () => {
    expect(sameCapture(DEFAULT_CAPTURE, { ...DEFAULT_CAPTURE })).toBe(true);
    expect(sameCapture(DEFAULT_CAPTURE, { ...DEFAULT_CAPTURE, neverSites: ["a.example"] })).toBe(
      false,
    );
    expect(sameCapture(DEFAULT_CAPTURE, { ...DEFAULT_CAPTURE, minSize: 0 })).toBe(false);
  });
});

describe("capture entries", () => {
  const cases = rulesCases as unknown as {
    sites: [string, string | null][];
    extensions: [string, string | null][];
  };

  it("normalise as the app and the extension do (rules-cases.json)", () => {
    for (const [typed, kept] of cases.sites) expect(normaliseSite(typed), typed).toBe(kept);
    for (const [typed, kept] of cases.extensions) {
      expect(normaliseExtension(typed), typed).toBe(kept);
    }
  });

  it("turn a name in another script into the browser's name for it", () => {
    expect(siteFromInput("bücher.example")).toBe("xn--bcher-kva.example");
    expect(siteFromInput("https://Bücher.example/x")).toBe("xn--bcher-kva.example");
    expect(siteFromInput("example.com")).toBe("example.com");
    expect(siteFromInput("not a site")).toBeNull();
  });
});

describe("capture fields", () => {
  it("read the smallest file in MiB", () => {
    expect(parseMinSize("1")).toEqual({ ok: true, value: 1 << 20 });
    expect(parseMinSize(" 0,5 ")).toEqual({ ok: true, value: 512 * 1024 });
    expect(parseMinSize("0")).toEqual({ ok: true, value: 0 });
    for (const bad of ["", "-1", "big", "1e3", String(2 ** 20 + 1)]) {
      expect(parseMinSize(bad)).toEqual({ ok: false, problem: t("settings.problem.capture_size") });
    }
    expect(formatMinSize(1 << 20)).toBe("1");
    expect(formatMinSize(1.5 * 2 ** 20)).toBe("1.5");
    expect(formatMinSize(0)).toBe("0");
  });

  it("read sites one per line, or with commas, and name the one that is wrong", () => {
    expect(parseSites("https://Bank.example/login\n*.corp.example, bank.example")).toEqual({
      ok: true,
      value: ["bank.example", "corp.example"],
    });
    expect(parseSites("  ")).toEqual({ ok: true, value: [] });
    expect(parseSites("ok.example\nuser@site")).toEqual({
      ok: false,
      problem: t("settings.problem.site_named", { site: "user@site" }),
    });
    const many = Array.from({ length: MAX_RULE_SITES + 1 }, (_, i) => `s${i}.example`);
    expect(parseSites(many.join("\n"))).toEqual({
      ok: false,
      problem: t("settings.problem.sites_many"),
    });
  });

  it("switch whole groups of types, keeping the order and the others", () => {
    const base = [...DEFAULT_CAPTURE.extensions, "epub"];
    const noVideo = setGroup(base, group("video"), false);
    expect(groupState(noVideo, group("video"))).toBe("off");
    expect(noVideo).not.toContain("mp4");
    expect(noVideo.at(-1)).toBe("epub");
    expect(setGroup(noVideo, group("video"), true)).toEqual(base);
    const docs = setGroup(base, group("documents"), true);
    expect(docs).toEqual([...DEFAULT_CAPTURE.extensions, ...group("documents").extensions, "epub"]);
    expect(groupState(["zip"], group("archives"))).toBe("mixed");
    expect(otherTypes(base)).toEqual(["epub"]);
  });

  it("read other types as typed, and let a grouped type join its group", () => {
    const base = setGroup(DEFAULT_CAPTURE.extensions, group("data"), false);
    expect(withOtherTypes(base, ".EPUB *.torrent, epub")).toEqual({
      ok: true,
      value: [...base, "epub", "torrent"],
    });
    const joined = withOtherTypes(base, "bin");
    expect(joined.ok && groupState(joined.value, group("data"))).toBe("mixed");
    expect(joined.ok && otherTypes(joined.value)).toEqual([]);
    // Clearing the field keeps the groups.
    expect(withOtherTypes([...base, "epub"], "")).toEqual({ ok: true, value: base });
    expect(withOtherTypes(base, "tar.gz")).toEqual({
      ok: false,
      problem: t("settings.problem.extension"),
    });
    const many = Array.from({ length: MAX_RULE_EXTENSIONS }, (_, i) => `x${i}`).join(" ");
    expect(withOtherTypes(base, many)).toEqual({
      ok: false,
      problem: t("settings.problem.capture_extensions_many"),
    });
  });
});
