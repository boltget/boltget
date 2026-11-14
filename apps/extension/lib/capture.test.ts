import { describe, expect, it } from "vitest";
import {
  baseName,
  extensionOf,
  isFetchable,
  isGenericMime,
  matchType,
  MAX_URL,
  shouldCapture,
  urlExtension,
  type CaptureEvent,
  type CaptureSettings,
  type Decision,
  type Modifiers,
} from "./capture";
import { DEFAULT_RULES } from "./rules";

const MIB = 1 << 20;

function settings(over: Partial<CaptureSettings> = {}): CaptureSettings {
  return { ...DEFAULT_RULES, active: true, ...over };
}

function event(over: Partial<CaptureEvent> = {}): CaptureEvent {
  return {
    url: "https://cdn.example.com/files/setup.exe",
    filename: "setup.exe",
    mime: "application/octet-stream",
    size: 50 * MIB,
    referrer: "https://www.example.com/download",
    incognito: false,
    ...over,
  };
}

describe("isFetchable", () => {
  it.each([
    ["https://example.com/a.zip", true],
    ["http://example.com/a.zip", true],
    ["HTTPS://EXAMPLE.COM/a.zip", true],
    ["blob:https://example.com/0a1b", false],
    ["data:application/zip;base64,UEsDBA==", false],
    ["file:///C:/Users/me/a.zip", false],
    ["ftp://example.com/a.zip", false],
    ["javascript:void(0)", false],
    ["about:blank", false],
    ["not a url", false],
    ["", false],
  ])("%s -> %s", (url, ok) => {
    expect(isFetchable(url)).toBe(ok);
  });

  it("refuses URLs over the protocol's 32 KiB", () => {
    const base = "https://example.com/?q=";
    expect(isFetchable(base + "a".repeat(MAX_URL - base.length))).toBe(true);
    expect(isFetchable(base + "a".repeat(MAX_URL - base.length + 1))).toBe(false);
  });

  it("refuses null and undefined", () => {
    expect(isFetchable(null)).toBe(false);
    expect(isFetchable(undefined)).toBe(false);
  });
});

describe("file names and extensions", () => {
  it("takes the last segment of either kind of path", () => {
    expect(baseName("C:\\Users\\me\\Downloads\\file.zip")).toBe("file.zip");
    expect(baseName("/home/me/Downloads/file.zip")).toBe("file.zip");
    expect(baseName("file.zip")).toBe("file.zip");
  });

  it.each([
    ["archive.ZIP", "zip"],
    ["movie.final.mkv", "mkv"],
    ["C:\\dl\\setup.exe", "exe"],
    ["linux.tar.gz", "gz"],
    ["noext", null],
    [".hidden", null],
    ["trailing.", null],
    ["download.php", null],
    ["page.html", null],
    ["weird.ex e", null],
    ["", null],
  ])("extensionOf(%s) = %s", (name, ext) => {
    expect(extensionOf(name)).toBe(ext);
  });

  it("reads the URL's path, not its query or fragment", () => {
    expect(urlExtension("https://e.com/a/b/file.iso?token=x.zip#y.rar")).toBe("iso");
    expect(urlExtension("https://e.com/get.php?file=a.zip")).toBe(null);
    expect(urlExtension("https://e.com/my%20file.7z")).toBe("7z");
    expect(urlExtension("https://e.com/")).toBe(null);
    expect(urlExtension("::")).toBe(null);
  });

  it("knows generic MIME types, parameters and case aside", () => {
    expect(isGenericMime("application/octet-stream")).toBe(true);
    expect(isGenericMime("Application/Octet-Stream; charset=binary")).toBe(true);
    expect(isGenericMime(null)).toBe(true);
    expect(isGenericMime("")).toBe(true);
    expect(isGenericMime("application/pdf")).toBe(false);
    expect(isGenericMime("text/html")).toBe(false);
  });
});

describe("matchType", () => {
  const list = DEFAULT_RULES.extensions;

  it("uses the file name first", () => {
    expect(matchType({ url: "https://e.com/a.pdf", filename: "a.zip" }, list)).toBe("match");
    expect(matchType({ url: "https://e.com/a.zip", filename: "a.pdf" }, list)).toBe("no_match");
  });

  it("then the final URL's path, then the link's", () => {
    expect(
      matchType({ url: "https://e.com/get?id=1", finalUrl: "https://cdn.e.com/x.msi" }, list),
    ).toBe("match");
    expect(
      matchType({ url: "https://e.com/x.pdf", finalUrl: "https://cdn.e.com/x.zip" }, list),
    ).toBe("match");
    expect(matchType({ url: "https://e.com/x.zip", finalUrl: "https://cdn.e.com/x" }, list)).toBe(
      "match",
    );
  });

  it("then the MIME type", () => {
    expect(matchType({ url: "https://e.com/get?id=1", mime: "video/mp4" }, list)).toBe("match");
    expect(matchType({ url: "https://e.com/get?id=1", mime: "application/pdf" }, list)).toBe(
      "no_match",
    );
    expect(
      matchType({ url: "https://e.com/get?id=1", mime: "application/x-7z-compressed" }, list),
    ).toBe("match");
  });

  it("is unknown when nothing names a type and the bytes are generic", () => {
    expect(
      matchType({ url: "https://e.com/download.php?id=1", mime: "application/octet-stream" }, list),
    ).toBe("unknown");
    expect(matchType({ url: "https://e.com/download" }, list)).toBe("unknown");
  });

  it("follows the list it is given", () => {
    expect(matchType({ url: "https://e.com/a.pdf", filename: "a.pdf" }, ["pdf"])).toBe("match");
    expect(matchType({ url: "https://e.com/a.zip", filename: "a.zip" }, [])).toBe("no_match");
    expect(matchType({ url: "https://e.com/get", mime: "video/mp4" }, ["zip"])).toBe("no_match");
  });
});

describe("shouldCapture: named cases", () => {
  it("captures a big installer", () => {
    expect(shouldCapture(event(), settings())).toEqual({
      capture: true,
      forced: false,
      reason: "type",
    });
  });

  it("captures a listed type whose size is unknown", () => {
    expect(shouldCapture(event({ size: null }), settings()).capture).toBe(true);
    expect(shouldCapture(event({ size: 0 }), settings()).capture).toBe(true);
    expect(shouldCapture(event({ size: -1 }), settings()).capture).toBe(true);
  });

  it("keeps a listed type under the threshold in the browser", () => {
    expect(shouldCapture(event({ size: MIB - 1 }), settings())).toEqual({
      capture: false,
      reason: "small",
    });
    expect(shouldCapture(event({ size: MIB }), settings()).capture).toBe(true);
  });

  it("keeps a 40 KB PDF, and a 40 MB one, in the browser by default", () => {
    const pdf = { url: "https://e.com/a.pdf", filename: "a.pdf", mime: "application/pdf" };
    expect(shouldCapture(event({ ...pdf, size: 40_000 }), settings()).reason).toBe("type");
    expect(shouldCapture(event({ ...pdf, size: 40 * MIB }), settings()).reason).toBe("type");
  });

  it("captures big generic bytes with no telling name, and not small ones", () => {
    const generic = {
      url: "https://e.com/download.php?id=7",
      filename: null,
      mime: "application/octet-stream",
    };
    expect(shouldCapture(event({ ...generic, size: 5 * MIB }), settings())).toEqual({
      capture: true,
      forced: false,
      reason: "size",
    });
    expect(shouldCapture(event({ ...generic, size: 5_000 }), settings()).reason).toBe("small");
    expect(shouldCapture(event({ ...generic, size: null }), settings()).reason).toBe("type");
  });

  it("never captures blob:, data: or file: downloads, even when forced", () => {
    for (const url of ["blob:https://e.com/1", "data:text/plain,hi", "file:///tmp/a.zip"]) {
      expect(shouldCapture(event({ url }), settings()).reason).toBe("unsupported_url");
      expect(shouldCapture(event({ url, forced: true }), settings()).reason).toBe(
        "unsupported_url",
      );
    }
  });

  it("never captures when redirects ended somewhere the app cannot fetch", () => {
    expect(shouldCapture(event({ finalUrl: "blob:https://e.com/1" }), settings()).reason).toBe(
      "unsupported_url",
    );
  });

  it("never captures downloads started by an extension (its own fallbacks included)", () => {
    expect(shouldCapture(event({ byExtension: true }), settings()).reason).toBe("extension");
    expect(shouldCapture(event({ byExtension: true, forced: true }), settings()).reason).toBe(
      "extension",
    );
  });

  it("never captures form submissions", () => {
    expect(shouldCapture(event({ method: "POST" }), settings()).reason).toBe("post");
    expect(shouldCapture(event({ method: "post", forced: true }), settings()).reason).toBe("post");
    expect(shouldCapture(event({ method: "GET" }), settings()).capture).toBe(true);
    expect(shouldCapture(event({ method: null }), settings()).capture).toBe(true);
  });

  it("keeps private windows out unless the rules allow them", () => {
    expect(shouldCapture(event({ incognito: true }), settings()).reason).toBe("private_window");
    expect(
      shouldCapture(event({ incognito: true }), settings({ private_windows: true })).capture,
    ).toBe(true);
  });

  it("lets an explicit request through in a private window", () => {
    expect(shouldCapture(event({ incognito: true, forced: true }), settings()).capture).toBe(true);
    expect(shouldCapture(event({ incognito: true, modifiers: "force" }), settings()).capture).toBe(
      true,
    );
  });

  it("Alt bypasses everything, the always list and forcing included", () => {
    expect(shouldCapture(event({ modifiers: "bypass" }), settings()).reason).toBe("bypass");
    expect(
      shouldCapture(
        event({ modifiers: "bypass", url: "https://dl.always.com/a.zip" }),
        settings({ always_sites: ["always.com"] }),
      ).reason,
    ).toBe("bypass");
  });

  it("Alt+Shift and the link menu force capture whatever the rules say", () => {
    const pdf = event({ filename: "a.pdf", url: "https://e.com/a.pdf", size: 10 });
    const off = settings({ enabled: false, active: false, never_sites: ["e.com"] });
    for (const forced of [{ modifiers: "force" as Modifiers }, { forced: true }]) {
      expect(shouldCapture({ ...pdf, ...forced }, off)).toEqual({
        capture: true,
        forced: true,
        reason: "forced",
      });
    }
  });

  it("respects the switch, the pause and the app's enabled", () => {
    expect(shouldCapture(event(), settings({ enabled: false })).reason).toBe("disabled");
    expect(shouldCapture(event(), settings({ active: false })).reason).toBe("disabled");
  });

  it("never captures from never-capture sites, by the download or by the page", () => {
    const never = settings({ never_sites: ["bank.example"] });
    expect(shouldCapture(event({ url: "https://bank.example/s.zip" }), never).reason).toBe(
      "never_site",
    );
    expect(shouldCapture(event({ url: "https://www.bank.example/s.zip" }), never).reason).toBe(
      "never_site",
    );
    expect(shouldCapture(event({ referrer: "https://online.bank.example/" }), never).reason).toBe(
      "never_site",
    );
    expect(
      shouldCapture(event({ finalUrl: "https://files.bank.example/s.zip" }), never).reason,
    ).toBe("never_site");
    // A suffix match is on labels, not characters.
    expect(shouldCapture(event({ url: "https://notbank.example/s.zip" }), never).capture).toBe(
      true,
    );
  });

  it("always captures from always-capture sites, whatever the type and size", () => {
    const always = settings({ always_sites: ["files.example"] });
    expect(
      shouldCapture(
        event({ url: "https://files.example/a.pdf", filename: "a.pdf", size: 10 }),
        always,
      ),
    ).toEqual({ capture: true, forced: false, reason: "always_site" });
  });

  it("the never list wins over the always list", () => {
    const both = settings({ never_sites: ["example.com"], always_sites: ["cdn.example.com"] });
    expect(shouldCapture(event(), both).reason).toBe("never_site");
  });

  it("a threshold of zero captures every listed type", () => {
    expect(shouldCapture(event({ size: 1 }), settings({ min_size: 0 })).capture).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Exhaustive: every combination of the inputs that matter, against an
// independent statement of the precedence in BROWSER-INTEGRATION.md.

type UrlKind = "https" | "blob" | "data" | "file";
type TypeKind = "listed" | "unlisted" | "generic";
type SizeKind = "unknown" | "small" | "big";
type SiteKind = "none" | "never" | "always" | "both";

interface Case {
  url: UrlKind;
  byExtension: boolean;
  method: "GET" | "POST" | null;
  incognito: boolean;
  privateWindows: boolean;
  modifiers: Modifiers;
  forced: boolean;
  enabled: boolean;
  active: boolean;
  site: SiteKind;
  type: TypeKind;
  size: SizeKind;
}

function oracle(c: Case): Decision {
  if (c.url !== "https") return { capture: false, reason: "unsupported_url" };
  if (c.byExtension) return { capture: false, reason: "extension" };
  if (c.method === "POST") return { capture: false, reason: "post" };
  if (c.modifiers === "bypass") return { capture: false, reason: "bypass" };
  if (c.forced || c.modifiers === "force") return { capture: true, forced: true, reason: "forced" };
  if (c.incognito && !c.privateWindows) return { capture: false, reason: "private_window" };
  if (!c.enabled || !c.active) return { capture: false, reason: "disabled" };
  if (c.site === "never" || c.site === "both") return { capture: false, reason: "never_site" };
  if (c.site === "always") return { capture: true, forced: false, reason: "always_site" };
  if (c.type === "listed") {
    return c.size === "small"
      ? { capture: false, reason: "small" }
      : { capture: true, forced: false, reason: "type" };
  }
  if (c.type === "generic" && c.size !== "unknown") {
    return c.size === "big"
      ? { capture: true, forced: false, reason: "size" }
      : { capture: false, reason: "small" };
  }
  return { capture: false, reason: "type" };
}

function build(c: Case): [CaptureEvent, CaptureSettings] {
  const host = { none: "e.com", never: "never.com", always: "always.com", both: "x.both.com" }[
    c.site
  ];
  const file = { listed: "a.zip", unlisted: "a.pdf", generic: "get" }[c.type];
  const https = `https://${host}/${file}`;
  const url = {
    https,
    blob: `blob:https://${host}/1`,
    data: "data:application/zip;base64,AA==",
    file: `file:///tmp/${file}`,
  }[c.url];
  const ev: CaptureEvent = {
    url,
    filename: c.type === "generic" ? null : file,
    mime: c.type === "generic" ? "application/octet-stream" : null,
    size: { unknown: null, small: 1000, big: 10 * MIB }[c.size],
    incognito: c.incognito,
    method: c.method,
    byExtension: c.byExtension,
    modifiers: c.modifiers,
    forced: c.forced,
  };
  const st = settings({
    enabled: c.enabled,
    active: c.active,
    private_windows: c.privateWindows,
    never_sites: ["never.com", "both.com"],
    always_sites: ["always.com", "x.both.com"],
  });
  return [ev, st];
}

function* cases(): Generator<Case> {
  const bools = [false, true];
  for (const url of ["https", "blob", "data", "file"] as UrlKind[])
    for (const byExtension of bools)
      for (const method of ["GET", "POST", null] as Case["method"][])
        for (const incognito of bools)
          for (const privateWindows of bools)
            for (const modifiers of ["none", "bypass", "force"] as Modifiers[])
              for (const forced of bools)
                for (const enabled of bools)
                  for (const active of bools)
                    for (const site of ["none", "never", "always", "both"] as SiteKind[])
                      for (const type of ["listed", "unlisted", "generic"] as TypeKind[])
                        for (const size of ["unknown", "small", "big"] as SizeKind[])
                          yield {
                            url,
                            byExtension,
                            method,
                            incognito,
                            privateWindows,
                            modifiers,
                            forced,
                            enabled,
                            active,
                            site,
                            type,
                            size,
                          };
}

describe("shouldCapture: every combination", () => {
  it("matches the documented precedence", () => {
    let n = 0;
    for (const c of cases()) {
      const [ev, st] = build(c);
      const got = shouldCapture(ev, st);
      if (JSON.stringify(got) !== JSON.stringify(oracle(c))) {
        expect({ case: c, got }).toEqual({ case: c, got: oracle(c) });
      }
      n++;
    }
    expect(n).toBe(4 * 2 * 3 * 2 * 2 * 3 * 2 * 2 * 2 * 4 * 3 * 3);
  });
});
