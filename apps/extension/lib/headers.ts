// Request header observation (docs/BROWSER-INTEGRATION.md, "Cookies, headers
// and referrer forwarding"): `webRequest.onSendHeaders` feeds a short-lived,
// size-capped log keyed by URL; a capture takes the entry for its URL and the
// allowlisted headers go to the app. Nothing here outlives 30 s.
import { ALLOWED_HEADERS } from "@boltget/native-messaging";

/** How long an observed request is kept (design target: 30 s). */
export const HEADER_TTL_MS = 30_000;
/** The most requests kept at once; the oldest go first. */
export const HEADER_LOG_CAP = 256;
/** The protocol's limit for a header value. */
export const MAX_HEADER_VALUE = 8 * 1024;

/** A header as `webRequest` reports it. */
export interface RawHeader {
  name: string;
  value?: string;
}

/** What was seen of one request. */
export interface ObservedRequest {
  url: string;
  method: string;
  headers: [string, string][];
  /** When it was seen, in ms. */
  at: number;
}

const ALLOWED = new Set<string>(ALLOWED_HEADERS);

/**
 * The allowlisted headers, each name once (the first wins), lowercase names,
 * values without CR or LF and within the protocol's limit.
 */
export function filterHeaders(headers: readonly RawHeader[] | undefined): [string, string][] {
  const out: [string, string][] = [];
  const seen = new Set<string>();
  for (const header of headers ?? []) {
    const name = header.name.toLowerCase();
    const value = header.value;
    if (!ALLOWED.has(name) || seen.has(name) || typeof value !== "string") continue;
    if (/[\r\n]/.test(value) || value.length > MAX_HEADER_VALUE || value === "") continue;
    seen.add(name);
    out.push([name, value]);
  }
  return out;
}

/** Recently observed requests, by URL. */
export class HeaderLog {
  readonly #entries = new Map<string, ObservedRequest>();
  readonly #ttl: number;
  readonly #cap: number;

  constructor(ttl = HEADER_TTL_MS, cap = HEADER_LOG_CAP) {
    this.#ttl = ttl;
    this.#cap = cap;
  }

  /** Records a request (a later one for the same URL replaces it). */
  record(url: string, method: string, headers: readonly RawHeader[] | undefined, now: number) {
    this.prune(now);
    this.#entries.delete(url);
    this.#entries.set(url, { url, method, headers: filterHeaders(headers), at: now });
    while (this.#entries.size > this.#cap) {
      const oldest = this.#entries.keys().next().value;
      if (oldest === undefined) break;
      this.#entries.delete(oldest);
    }
  }

  /**
   * The request for the first of `urls` that was seen (the download's final
   * URL, then its link), removed from the log: it is used once.
   */
  take(urls: (string | null | undefined)[], now: number): ObservedRequest | null {
    this.prune(now);
    for (const url of urls) {
      if (!url) continue;
      const entry = this.#entries.get(url);
      if (entry) {
        this.#entries.delete(url);
        return entry;
      }
    }
    return null;
  }

  /** Drops what is older than the time to live. */
  prune(now: number) {
    for (const [url, entry] of this.#entries) {
      if (now - entry.at > this.#ttl) this.#entries.delete(url);
    }
  }

  get size(): number {
    return this.#entries.size;
  }
}
