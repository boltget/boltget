// The browser capture rules in Settings, Browser (docs/BROWSER-INTEGRATION.md,
// "Capture rules"): their defaults, the file-type groups the page offers,
// and the checks a typed value gets before it is sent. Entries normalise as
// `normalise_site` and `normalise_extension` in crates/ipc/src/browser.rs do
// (the backend normalises and checks again), and as the extension's
// lib/rules.ts does: crates/ipc/src/browser/rules-cases.json holds all three
// to the same answers.
import { t, type PlainMessageKey } from "@boltget/ui/messages";
import type { CaptureSettings } from "../bindings";
import type { Check } from "./model";

const MIB = 1024 * 1024;

/** The largest "smallest file": 1 TiB (`MAX_MIN_SIZE`). */
export const MAX_MIN_SIZE = 2 ** 40;
/** The most file types (`MAX_RULE_EXTENSIONS`). */
export const MAX_RULE_EXTENSIONS = 200;
/** The most sites in one list (`MAX_RULE_SITES`). */
export const MAX_RULE_SITES = 500;
const MAX_SITE = 253;

const ok = <T>(value: T): Check<T> => ({ ok: true, value });
const bad = (problem: string): Check<never> => ({ ok: false, problem });

export type TypeGroupId =
  "archives" | "discs" | "programs" | "video" | "audio" | "data" | "documents";

export interface TypeGroup {
  id: TypeGroupId;
  label: PlainMessageKey;
  extensions: readonly string[];
  /** Part of the defaults. */
  byDefault: boolean;
}

/**
 * The groups of file types the page offers, in order. The default groups,
 * in this order, are exactly the protocol's `DEFAULT_EXTENSIONS` (a test
 * reads it); documents are offered but off, as the spec says.
 */
export const TYPE_GROUPS: readonly TypeGroup[] = [
  {
    id: "archives",
    label: "settings.capture_types.archives",
    extensions: ["zip", "rar", "7z", "tar", "gz", "bz2", "xz", "zst"],
    byDefault: true,
  },
  {
    id: "discs",
    label: "settings.capture_types.discs",
    extensions: ["iso", "img"],
    byDefault: true,
  },
  {
    id: "programs",
    label: "settings.capture_types.programs",
    extensions: ["exe", "msi", "msix", "appx", "dmg", "pkg", "deb", "rpm", "apk"],
    byDefault: true,
  },
  {
    id: "video",
    label: "settings.capture_types.video",
    extensions: ["mp4", "mkv", "webm", "mov", "avi", "m4v"],
    byDefault: true,
  },
  {
    id: "audio",
    label: "settings.capture_types.audio",
    extensions: ["mp3", "flac", "wav", "m4a", "ogg", "opus"],
    byDefault: true,
  },
  {
    id: "data",
    label: "settings.capture_types.data",
    extensions: ["bin", "dat"],
    byDefault: true,
  },
  {
    id: "documents",
    label: "settings.capture_types.documents",
    extensions: ["pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf"],
    byDefault: false,
  },
];

const GROUPED = new Set(TYPE_GROUPS.flatMap((g) => g.extensions));

/** The rules the app starts with (`CaptureRules::default()`). */
export const DEFAULT_CAPTURE: CaptureSettings = {
  enabled: true,
  minSize: MIB,
  extensions: TYPE_GROUPS.filter((g) => g.byDefault).flatMap((g) => [...g.extensions]),
  neverSites: [],
  alwaysSites: [],
  privateWindows: false,
};

/** Whether two sets of rules are the same. */
export function sameCapture(a: CaptureSettings, b: CaptureSettings): boolean {
  const list = (x: string[], y: string[]) => x.length === y.length && x.every((v, i) => v === y[i]);
  return (
    a.enabled === b.enabled &&
    a.minSize === b.minSize &&
    a.privateWindows === b.privateWindows &&
    list(a.extensions, b.extensions) &&
    list(a.neverSites, b.neverSites) &&
    list(a.alwaysSites, b.alwaysSites)
  );
}

// Entries ----------------------------------------------------------------

const ASCII = /^[\x20-\x7e]*$/;

/** A file type as the rules keep it (`normalise_extension`), or null. */
export function normaliseExtension(raw: string): string | null {
  const s = raw.trim();
  if (!ASCII.test(s)) return null;
  const ext = s.toLowerCase().replace(/^\*?\./, "");
  return /^[a-z0-9]{1,16}$/.test(ext) ? ext : null;
}

/** A site as the lists keep it (`normalise_site`), or null. */
export function normaliseSite(raw: string): string | null {
  let site = raw.trim();
  if (!ASCII.test(site)) return null;
  site = site.toLowerCase();
  site = site.replace(/^[a-z][a-z0-9+.-]*:\/\//, "");
  site = site.replace(/[/?#][\s\S]*$/, "");
  site = site.replace(/:\d+$/, "");
  site = site.replace(/^\*?\.+/, "").replace(/\.+$/, "");
  if (site === "" || site.length > MAX_SITE || !/^[a-z0-9.-]+$/.test(site) || site.includes("..")) {
    return null;
  }
  return site;
}

/**
 * A site as typed in Settings: like `normaliseSite`, and a name in another
 * script (`bücher.example`) becomes the name browsers use for it
 * (`xn--bcher-kva.example`).
 */
export function siteFromInput(raw: string): string | null {
  const direct = normaliseSite(raw);
  if (direct !== null || ASCII.test(raw.trim())) return direct;
  const typed = raw.trim();
  try {
    const url = new URL(typed.includes("://") ? typed : `http://${typed}`);
    return normaliseSite(url.hostname);
  } catch {
    return null;
  }
}

/** Splits a typed list: spaces, commas, semicolons or lines between entries. */
export function splitEntries(text: string): string[] {
  return text.split(/[\s,;]+/).filter(Boolean);
}

// Fields ------------------------------------------------------------------

/** "Smallest file" in MiB (a comma works as the decimal point); 0 is any size. */
export function parseMinSize(text: string): Check<number> {
  const s = text.trim().replace(",", ".");
  if (!/^\d*\.?\d+$/.test(s)) return bad(t("settings.problem.capture_size"));
  const bytes = Math.round(Number(s) * MIB);
  return bytes <= MAX_MIN_SIZE ? ok(bytes) : bad(t("settings.problem.capture_size"));
}

/** A size as the field shows it: MiB without trailing zeros. */
export function formatMinSize(bytes: number): string {
  return String(Math.round((bytes / MIB) * 100) / 100);
}

/** A typed list of sites, normalised, without duplicates. */
export function parseSites(text: string): Check<string[]> {
  const out: string[] = [];
  for (const raw of splitEntries(text)) {
    const site = siteFromInput(raw);
    if (site === null) {
      const shown = raw.length > 64 ? `${raw.slice(0, 64)}…` : raw;
      return bad(t("settings.problem.site_named", { site: shown }));
    }
    if (!out.includes(site)) out.push(site);
  }
  if (out.length > MAX_RULE_SITES) return bad(t("settings.problem.sites_many"));
  return ok(out);
}

// File types ----------------------------------------------------------------

export type GroupState = "on" | "off" | "mixed";

/** Whether all, none or some of a group's types are captured. */
export function groupState(extensions: readonly string[], group: TypeGroup): GroupState {
  const n = group.extensions.filter((e) => extensions.includes(e)).length;
  return n === 0 ? "off" : n === group.extensions.length ? "on" : "mixed";
}

/** The captured types that are in no group, in their order. */
export function otherTypes(extensions: readonly string[]): string[] {
  return extensions.filter((e) => !GROUPED.has(e));
}

/** The types in the page's order: by group, then the others. */
function ordered(set: ReadonlySet<string>, others: readonly string[]): string[] {
  const grouped = TYPE_GROUPS.flatMap((g) => g.extensions.filter((e) => set.has(e)));
  const rest = others.filter((e) => !GROUPED.has(e));
  return [...new Set([...grouped, ...rest])];
}

/** The types with a whole group switched on or off. */
export function setGroup(extensions: readonly string[], group: TypeGroup, on: boolean): string[] {
  const set = new Set(extensions);
  for (const e of group.extensions) {
    if (on) set.add(e);
    else set.delete(e);
  }
  return ordered(set, otherTypes(extensions));
}

/**
 * The types with "Other types" as typed: the grouped ones stay as they are; a
 * typed type that belongs to a group joins it.
 */
export function withOtherTypes(extensions: readonly string[], text: string): Check<string[]> {
  const typed: string[] = [];
  for (const raw of splitEntries(text)) {
    const ext = normaliseExtension(raw);
    if (ext === null) return bad(t("settings.problem.extension"));
    typed.push(ext);
  }
  const set = new Set([...extensions.filter((e) => GROUPED.has(e)), ...typed]);
  const out = ordered(set, typed);
  if (out.length > MAX_RULE_EXTENSIONS) return bad(t("settings.problem.capture_extensions_many"));
  return ok(out);
}
