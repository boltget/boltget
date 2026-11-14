// Capture rules: the app's defaults, normalising what the user or the app
// gives us, and host-suffix matching (docs/BROWSER-INTEGRATION.md, "Capture
// rules").
import type { CaptureRules } from "@boltget/native-messaging";

/**
 * The app's default file types, the same list as `DEFAULT_EXTENSIONS` in
 * crates/ipc/src/browser.rs (a test keeps the two in step). Used until the app
 * has answered a `hello` with its own rules.
 */
export const DEFAULT_EXTENSIONS = [
  "zip",
  "rar",
  "7z",
  "tar",
  "gz",
  "bz2",
  "xz",
  "zst",
  "iso",
  "img",
  "exe",
  "msi",
  "msix",
  "appx",
  "dmg",
  "pkg",
  "deb",
  "rpm",
  "apk",
  "mp4",
  "mkv",
  "webm",
  "mov",
  "avi",
  "m4v",
  "mp3",
  "flac",
  "wav",
  "m4a",
  "ogg",
  "opus",
  "bin",
  "dat",
] as const;

/** 1 MiB: small files are faster in the browser than any handoff. */
export const DEFAULT_MIN_SIZE = 1 << 20;

/** The rules the app answers `hello` with by default. */
export const DEFAULT_RULES: CaptureRules = {
  enabled: true,
  min_size: DEFAULT_MIN_SIZE,
  extensions: [...DEFAULT_EXTENSIONS],
  never_sites: [],
  always_sites: [],
  private_windows: false,
};

const EXTENSION = /^[a-z0-9]{1,16}$/;

/** A file extension as the rules store it: lowercase, without the dot. */
export function normaliseExtension(raw: string): string | null {
  const ext = raw
    .trim()
    .toLowerCase()
    .replace(/^\*?\./, "");
  return EXTENSION.test(ext) ? ext : null;
}

/**
 * A site as the lists store it: a lowercase host, from whatever the user
 * typed (`https://Example.com/path`, `*.example.com`, `.example.com`).
 */
export function normaliseSite(raw: string): string | null {
  let site = raw.trim().toLowerCase();
  if (site === "") return null;
  site = site.replace(/^[a-z][a-z0-9+.-]*:\/\//, "");
  site = site.replace(/[/?#].*$/, "");
  site = site.replace(/:\d+$/, "");
  site = site.replace(/^\*?\.+/, "").replace(/\.+$/, "");
  if (site === "" || !/^[a-z0-9.-]+$/.test(site) || site.includes("..")) return null;
  return site;
}

function unique(values: Iterable<string | null>): string[] {
  const out: string[] = [];
  for (const v of values) if (v !== null && !out.includes(v)) out.push(v);
  return out;
}

function strings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((v): v is string => typeof v === "string") : [];
}

/**
 * Rules from storage or from the app, made safe to apply: unknown or broken
 * fields fall back to the defaults, lists are normalised and deduplicated.
 */
export function normaliseRules(value: unknown): CaptureRules {
  const raw = (typeof value === "object" && value !== null ? value : {}) as Record<string, unknown>;
  const minSize = raw.min_size;
  return {
    enabled: typeof raw.enabled === "boolean" ? raw.enabled : DEFAULT_RULES.enabled,
    min_size:
      typeof minSize === "number" && Number.isFinite(minSize) && minSize >= 0
        ? Math.floor(minSize)
        : DEFAULT_RULES.min_size,
    extensions: Array.isArray(raw.extensions)
      ? unique(strings(raw.extensions).map(normaliseExtension))
      : [...DEFAULT_RULES.extensions],
    never_sites: unique(strings(raw.never_sites).map(normaliseSite)),
    always_sites: unique(strings(raw.always_sites).map(normaliseSite)),
    private_windows:
      typeof raw.private_windows === "boolean"
        ? raw.private_windows
        : DEFAULT_RULES.private_windows,
  };
}

/** Splits a text field (commas, spaces or lines) into list entries. */
export function splitList(text: string): string[] {
  return text.split(/[\s,;]+/).filter((s) => s !== "");
}

/** Whether `host` is `site` or one of its subdomains. */
export function hostMatches(host: string, site: string): boolean {
  const h = host.toLowerCase().replace(/\.$/, "");
  return h === site || h.endsWith(`.${site}`);
}

/** The host of a URL, or null when it has none or does not parse. */
export function hostOf(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    const host = new URL(url).hostname;
    return host === "" ? null : host.toLowerCase();
  } catch {
    return null;
  }
}

/** Whether any of the URLs is on one of the sites. */
export function onAnySite(urls: (string | null | undefined)[], sites: string[]): boolean {
  if (sites.length === 0) return false;
  return urls.some((url) => {
    const host = hostOf(url);
    return host !== null && sites.some((site) => hostMatches(host, site));
  });
}
