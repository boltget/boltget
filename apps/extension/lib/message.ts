// Builds the protocol's messages from what the browser gave us, within the
// host's limits (BROWSER-INTEGRATION.md, "Strict and bounded"): a value the
// host would refuse is left out rather than sent, because one bad field
// refuses the whole message and the download would stay in the browser.
import type { AddLinks, Capture, CookieRecord, Link, MediaPage } from "@boltget/native-messaging";
import { baseName, isFetchable } from "./capture";

/** The protocol's limits. */
export const MAX_TEXT = 1024;
export const MAX_MIME = 255;
export const MAX_LINKS = 5000;

// eslint-disable-next-line no-control-regex -- the protocol forbids control characters
const CONTROL = /[\u0000-\u001f\u007f]/g;

function bytes(text: string): number {
  return new TextEncoder().encode(text).length;
}

/** Cuts text to at most `max` UTF-8 bytes without splitting a character. */
export function clip(text: string, max = MAX_TEXT): string {
  if (bytes(text) <= max) return text;
  let out = "";
  for (const ch of text) {
    if (bytes(out + ch) > max) break;
    out += ch;
  }
  return out;
}

/** A file name the host accepts: no folders, no control characters, at most 1 KiB. */
export function cleanFilename(name: string | null | undefined): string | null {
  if (!name) return null;
  const base = clip(baseName(name).replace(CONTROL, "").trim());
  return base === "" || base === "." || base === ".." ? null : base;
}

/** A MIME type the host accepts: printable ASCII, at most 255 characters. */
export function cleanMime(mime: string | null | undefined): string | null {
  if (!mime) return null;
  const m = mime.trim();
  return m !== "" && m.length <= MAX_MIME && /^[\x20-\x7e]+$/.test(m) ? m : null;
}

/** Display text (link texts, titles): one line, at most 1 KiB, or null. */
export function cleanText(text: string | null | undefined): string | null {
  if (!text) return null;
  const t = clip(text.replace(CONTROL, " ").replace(/\s+/g, " ").trim());
  return t === "" ? null : t;
}

function url(value: string | null | undefined): string | null {
  return isFetchable(value) ? value : null;
}

/** Everything that goes into a `capture`. */
export interface CaptureInput {
  url: string;
  finalUrl?: string | null;
  filename?: string | null;
  size?: number | null;
  mime?: string | null;
  referrer?: string | null;
  pageUrl?: string | null;
  incognito: boolean;
  forced: boolean;
  cookies: CookieRecord[];
  headers: [string, string][];
}

/** The `capture` message's fields. */
export function captureMessage(input: CaptureInput): Capture {
  const message: Capture = {
    url: input.url,
    incognito: input.incognito,
    forced: input.forced,
    cookies: input.cookies,
    headers: input.headers,
  };
  const finalUrl = url(input.finalUrl);
  if (finalUrl && finalUrl !== input.url) message.final_url = finalUrl;
  const filename = cleanFilename(input.filename);
  if (filename) message.filename = filename;
  if (typeof input.size === "number" && Number.isSafeInteger(input.size) && input.size > 0) {
    message.size = input.size;
  }
  const mime = cleanMime(input.mime);
  if (mime) message.mime = mime;
  const pageUrl = url(input.pageUrl);
  const referrer = url(input.referrer) ?? pageUrl;
  if (referrer) message.referrer = referrer;
  if (pageUrl) message.page_url = pageUrl;
  return message;
}

/** A link as the page reported it (content script). */
export interface RawLink {
  url: string;
  text?: string | null;
}

/** The page's links as the protocol takes them: `http`/`https`, unique, at most 5000. */
export function cleanLinks(raw: readonly RawLink[]): Link[] {
  const out: Link[] = [];
  const seen = new Set<string>();
  for (const link of raw) {
    if (out.length >= MAX_LINKS) break;
    if (!isFetchable(link.url)) continue;
    // The fragment never reaches the server: the same file.
    const key = link.url.replace(/#.*$/, "");
    if (seen.has(key)) continue;
    seen.add(key);
    const text = cleanText(link.text);
    out.push(text ? { url: key, text } : { url: key });
  }
  return out;
}

/** The `add_links` message's fields, or null when no link is left. */
export function addLinksMessage(
  links: readonly RawLink[],
  pageUrl: string | null | undefined,
  incognito: boolean,
  cookies: CookieRecord[],
  headers: [string, string][],
): AddLinks | null {
  const clean = cleanLinks(links);
  if (clean.length === 0) return null;
  const message: AddLinks = { links: clean, incognito, cookies, headers };
  const page = url(pageUrl);
  if (page) message.page_url = page;
  return message;
}

/** The `media_page` message's fields, or null for a page the app cannot fetch. */
export function mediaPageMessage(
  pageUrl: string | null | undefined,
  title: string | null | undefined,
  incognito: boolean,
  cookies: CookieRecord[],
): MediaPage | null {
  const page = url(pageUrl);
  if (!page) return null;
  const message: MediaPage = { page_url: page, incognito, cookies, headers: [] };
  const t = cleanText(title);
  if (t) message.title = t;
  return message;
}
