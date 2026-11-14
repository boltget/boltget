// Cookies for a captured download (docs/BROWSER-INTEGRATION.md, "Cookies,
// headers and referrer forwarding"): `cookies.getAll` for the link and the
// final URL, also from the page's partition (CHIPS) and, on Firefox, across
// first-party isolation, converted to the protocol's records within its limits.
import type { CookieRecord, SameSite } from "@boltget/native-messaging";

/** A cookie as `cookies.getAll` reports it (the fields we read). */
export interface BrowserCookie {
  name: string;
  value: string;
  domain: string;
  path: string;
  hostOnly: boolean;
  secure: boolean;
  httpOnly: boolean;
  sameSite?: string;
  session?: boolean;
  expirationDate?: number;
  partitionKey?: unknown;
  firstPartyDomain?: string;
}

/** `cookies.getAll`'s query, with the fields only some browsers know. */
export interface CookieQuery {
  url: string;
  storeId?: string;
  partitionKey?: { topLevelSite?: string };
  firstPartyDomain?: string | null;
}

/** The cookie API, injected so the logic runs without a browser. */
export type GetAllCookies = (query: CookieQuery) => Promise<BrowserCookie[]>;

/** The protocol's limits. */
export const MAX_COOKIES = 300;
export const MAX_COOKIE_BYTES = 64 * 1024;

// eslint-disable-next-line no-control-regex -- the protocol forbids control characters
const FORBIDDEN = /[;\u0000-\u001f\u007f]/;

const utf8 = new TextEncoder();

const SAME_SITE: Record<string, SameSite> = {
  no_restriction: "no_restriction",
  lax: "lax",
  strict: "strict",
  unspecified: "unspecified",
};

/** One browser cookie as a protocol record, or null when the host would refuse it. */
export function toRecord(cookie: BrowserCookie): CookieRecord | null {
  if (FORBIDDEN.test(cookie.name) || FORBIDDEN.test(cookie.value)) return null;
  if (!cookie.domain || !/^\.?[A-Za-z0-9.-]+$/.test(cookie.domain)) return null;
  const path = cookie.path?.startsWith("/") ? cookie.path : "/";
  const record: CookieRecord = {
    name: cookie.name,
    value: cookie.value,
    domain: cookie.domain,
    path,
    host_only: cookie.hostOnly,
    secure: cookie.secure,
    http_only: cookie.httpOnly,
  };
  const sameSite = cookie.sameSite ? SAME_SITE[cookie.sameSite] : undefined;
  if (sameSite) record.same_site = sameSite;
  if (!cookie.session && typeof cookie.expirationDate === "number") {
    record.expires = Math.floor(cookie.expirationDate);
  }
  return record;
}

/** The top-level site (`scheme://registrable-ish host`) for a partition key. */
export function topLevelSite(pageUrl: string | null | undefined): string | null {
  if (!pageUrl) return null;
  try {
    const url = new URL(pageUrl);
    if (url.protocol !== "https:" && url.protocol !== "http:") return null;
    return `${url.protocol}//${url.hostname}`;
  } catch {
    return null;
  }
}

/** Where to look and what the browser supports. */
export interface CookieOptions {
  storeId?: string;
  /** The tab's URL, for partitioned cookies. */
  pageUrl?: string | null;
  /** Firefox: query across first-party isolation (`firstPartyDomain: null`). */
  firefox?: boolean;
}

/**
 * Every cookie the browser would send to `urls`, deduplicated, within the
 * protocol's limits. Failures (no host permission, an older browser that does
 * not know a query field) give fewer cookies, never an error: capture still
 * works without them.
 */
export async function collectCookies(
  getAll: GetAllCookies,
  urls: (string | null | undefined)[],
  options: CookieOptions = {},
): Promise<CookieRecord[]> {
  const site = topLevelSite(options.pageUrl);
  const queries: CookieQuery[] = [];
  for (const url of new Set(urls.filter((u): u is string => !!u))) {
    const base: CookieQuery = { url };
    if (options.storeId) base.storeId = options.storeId;
    if (options.firefox) base.firstPartyDomain = null;
    queries.push(base);
    if (site) queries.push({ ...base, partitionKey: { topLevelSite: site } });
  }

  const results = await Promise.all(queries.map((q) => getAll(q).catch(() => [])));
  const out: CookieRecord[] = [];
  const seen = new Set<string>();
  let bytes = 0;
  for (const cookie of results.flat()) {
    const key = `${cookie.name}\u0000${cookie.domain}\u0000${cookie.path}\u0000${JSON.stringify(cookie.partitionKey ?? null)}`;
    if (seen.has(key)) continue;
    seen.add(key);
    const record = toRecord(cookie);
    if (!record) continue;
    const size = utf8.encode(record.name).length + utf8.encode(record.value).length;
    if (out.length >= MAX_COOKIES || bytes + size > MAX_COOKIE_BYTES) break;
    bytes += size;
    out.push(record);
  }
  return out;
}
