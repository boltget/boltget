// The add sheet's rules, kept free of Svelte so they are easy to test:
// which links a text holds, whether the options typed are ones the engine
// accepts, and whether a link is already in the list (APP-DESIGN #6 to #8).
import type { DownloadId, DownloadState } from "../bindings";

const MIB = 1024 * 1024;
/** The engine's per-download range (docs/ENGINE.md, "Limits"). */
export const MAX_CONNECTIONS = 32;

/** The http and https links in a pasted or dropped text, in order, without repeats. */
export function linksIn(text: string): string[] {
  const found = new Set<string>();
  for (const word of text.split(/\s+/)) {
    const candidate = word.replace(/^[<("'[]+|[>)"',.;\]]+$/g, "");
    if (!/^https?:\/\//i.test(candidate)) continue;
    try {
      const url = new URL(candidate);
      if ((url.protocol === "http:" || url.protocol === "https:") && url.hostname) {
        found.add(url.href);
      }
    } catch {
      // Not a link.
    }
  }
  return [...found];
}

/** What the link field holds: nothing, text that is no link, or links. */
export type LinkInput = { kind: "empty" } | { kind: "invalid" } | { kind: "links"; urls: string[] };

export function readLinks(text: string): LinkInput {
  if (!text.trim()) return { kind: "empty" };
  const urls = linksIn(text);
  return urls.length ? { kind: "links", urls } : { kind: "invalid" };
}

// Options ---------------------------------------------------------------

/** A parsed option, or the catalog key of what is wrong with it. */
export type Parsed<T> = { ok: true; value: T } | { ok: false; problem: AddProblem };

export type AddProblem =
  | "add.problem.connections"
  | "add.problem.limit"
  | "add.problem.checksum"
  | "add.problem.name"
  | "add.problem.name_folder";

const ok = <T>(value: T): Parsed<T> => ({ ok: true, value });
const bad = (problem: AddProblem): Parsed<never> => ({ ok: false, problem });

/** Connections: empty is Auto (`null`), otherwise 1 to 32. */
export function parseConnections(text: string): Parsed<number | null> {
  const s = text.trim();
  if (!s || s.toLowerCase() === "auto") return ok(null);
  if (!/^\d+$/.test(s)) return bad("add.problem.connections");
  const n = Number(s);
  return n >= 1 && n <= MAX_CONNECTIONS ? ok(n) : bad("add.problem.connections");
}

/** A speed limit typed in MiB/s: empty is no limit; returns bytes per second. */
export function parseLimit(text: string): Parsed<number | null> {
  const s = text.trim().replace(",", ".");
  if (!s) return ok(null);
  if (!/^\d*\.?\d+$/.test(s)) return bad("add.problem.limit");
  const value = Number(s);
  const bytes = Math.round(value * MIB);
  return value > 0 && bytes > 0 ? ok(bytes) : bad("add.problem.limit");
}

const HEX_LENGTHS: Record<string, number> = { md5: 32, sha1: 40, sha256: 64, sha512: 128 };

/**
 * A checksum as the engine's `Checksum::parse` takes it: `algo:hex`,
 * `algo=hex` or bare hex (the algorithm from its length). Empty is none.
 * Returns the text to send, trimmed.
 */
export function parseChecksum(text: string): Parsed<string | null> {
  const s = text.trim();
  if (!s) return ok(null);
  const split = s.search(/[:=]/);
  let algo: string | undefined;
  let hex = s;
  if (split >= 0) {
    const name = s.slice(0, split).replace(/[-_]/g, "").toLowerCase().replace(/^sha$/, "sha1");
    if (!(name in HEX_LENGTHS)) return bad("add.problem.checksum");
    algo = name;
    hex = s.slice(split + 1).trim();
  }
  if (!/^[0-9a-f]+$/i.test(hex)) return bad("add.problem.checksum");
  const fits = algo
    ? HEX_LENGTHS[algo] === hex.length
    : Object.values(HEX_LENGTHS).includes(hex.length);
  return fits ? ok(algo ? `${algo}:${hex}` : hex) : bad("add.problem.checksum");
}

/** A typed file name: not empty, and no folder in it. */
export function parseFileName(text: string): Parsed<string> {
  const s = text.trim();
  if (!s || s === "." || s === "..") return bad("add.problem.name");
  if (/[\\/:]/.test(s)) return bad("add.problem.name_folder");
  return ok(s);
}

// Duplicates ------------------------------------------------------------

/** What the list knows about a download, for the duplicate check. */
export interface Listed {
  id: DownloadId;
  url: string;
  state: DownloadState;
  completedAt: number | null;
}

export type Duplicate =
  | { kind: "downloaded"; id: DownloadId; completedAt: number | null }
  | { kind: "listed"; id: DownloadId };

/**
 * The link without what the list leaves out (query, fragment, credentials):
 * rows keep links redacted, so two links compare by origin and path.
 */
export function linkKey(url: string): string | null {
  try {
    const u = new URL(url);
    return `${u.protocol}//${u.host}${u.pathname}`;
  } catch {
    return null;
  }
}

/**
 * Whether `url` is already in the list: a finished download of it wins
 * (the newest), then any other row with it.
 */
export function findDuplicate(url: string, items: readonly Listed[]): Duplicate | null {
  const key = linkKey(url);
  if (key === null) return null;
  let listed: Listed | undefined;
  let done: Listed | undefined;
  for (const item of items) {
    if (linkKey(item.url) !== key) continue;
    if (item.state === "completed") {
      if (!done || (item.completedAt ?? 0) > (done.completedAt ?? 0)) done = item;
    } else listed ??= item;
  }
  if (done) return { kind: "downloaded", id: done.id, completedAt: done.completedAt };
  if (listed) return { kind: "listed", id: listed.id };
  return null;
}
