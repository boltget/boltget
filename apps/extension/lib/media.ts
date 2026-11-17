// Media detection (docs/BROWSER-INTEGRATION.md, "Media detection panel"): the
// extension only notices media a page loads and hands the page to the app,
// which runs yt-dlp on it. Nothing here knows any site.
//
// - Responses (`webRequest.onHeadersReceived`, with site access only) that are
//   an HLS or DASH manifest, or a video or audio file, become hints for their
//   tab: classifyResponse.
// - Each tab keeps at most MAX_TAB_HINTS, once per link, cleared when the tab
//   moves to another page: TabMedia and addHint. Segments of a stream are
//   never listed; once a tab has a manifest, its files are taken for
//   segments and dropped.
// - The popup looks at the page itself when it opens (scanPage, injected once
//   through activeTab: no content script stays on any page).
import type { MediaHint, MediaKind } from "@boltget/native-messaging";
import { MAX_HINTS } from "@boltget/native-messaging";
import { isFetchable } from "./capture";
import { cleanMime } from "./message";

/** The most hints a tab keeps (the protocol's limit for one `media_page`). */
export const MAX_TAB_HINTS = MAX_HINTS;

/** Files smaller than this are not media worth listing (design target: 512 KiB). */
export const MIN_MEDIA_SIZE = 512 * 1024;

/** A header as `webRequest` reports it. */
export interface ResponseHeader {
  name: string;
  value?: string;
}

/** What `onHeadersReceived` tells of a response. */
export interface ResponseInfo {
  url: string;
  statusCode: number;
  method?: string;
  responseHeaders?: readonly ResponseHeader[];
}

const HLS_TYPES = new Set([
  "application/vnd.apple.mpegurl",
  "application/x-mpegurl",
  "audio/mpegurl",
  "audio/x-mpegurl",
]);
const DASH_TYPES = new Set(["application/dash+xml"]);

/** Content types that say nothing about what the bytes are. */
const GENERIC_TYPES = new Set([
  "",
  "application/octet-stream",
  "binary/octet-stream",
  "application/binary",
  "application/x-binary",
  "text/plain",
]);

/** Pieces of a stream rather than something to download on its own. */
const SEGMENT_TYPES = new Set(["video/mp2t", "video/iso.segment", "audio/iso.segment"]);
const SEGMENT_EXTENSIONS = new Set(["ts", "m4s", "m4f", "cmfv", "cmfa", "aac", "vtt", "webvtt"]);

/** File extensions of video and audio files, for responses with a generic type. */
const FILE_EXTENSIONS = new Set([
  "mp4",
  "m4v",
  "webm",
  "mkv",
  "mov",
  "m4a",
  "mp3",
  "ogg",
  "oga",
  "opus",
  "flac",
  "wav",
]);

function header(headers: readonly ResponseHeader[] | undefined, name: string): string | null {
  const h = headers?.find((x) => x.name.toLowerCase() === name);
  return typeof h?.value === "string" ? h.value : null;
}

/** The lowercase MIME type without parameters. */
function mimeOf(value: string | null): string {
  return (value ?? "").split(";")[0]?.trim().toLowerCase() ?? "";
}

/** The lowercase extension of a link's path, or "". */
export function pathExtension(url: string): string {
  try {
    const path = new URL(url).pathname;
    const last = path.slice(path.lastIndexOf("/") + 1);
    const dot = last.lastIndexOf(".");
    return dot > 0 ? last.slice(dot + 1).toLowerCase() : "";
  } catch {
    return "";
  }
}

/**
 * A response's whole size: the total of a `Content-Range` (a player's range
 * request), else its `Content-Length`; null when neither says.
 */
export function responseSize(headers: readonly ResponseHeader[] | undefined): number | null {
  const range = header(headers, "content-range");
  const total = range ? /\/\s*(\d+)\s*$/.exec(range)?.[1] : undefined;
  const raw = total ?? header(headers, "content-length");
  if (raw === null || raw === undefined || !/^\d+$/.test(raw.trim())) return null;
  const n = Number(raw.trim());
  return Number.isSafeInteger(n) ? n : null;
}

/** A hint as the background keeps it: the protocol's shape. */
export type Hint = MediaHint;

/**
 * Whether a response is media worth a hint, and what kind: a manifest by its
 * type (or its `.m3u8`/`.mpd` path when the type says nothing), a video or
 * audio file by its type (or its extension when the type says nothing), not
 * a segment of a stream, and not a file known to be smaller than
 * MIN_MEDIA_SIZE. Only successful `GET`s of `http(s)` links.
 */
export function classifyResponse(r: ResponseInfo): Hint | null {
  if (!isFetchable(r.url)) return null;
  if (r.method && r.method.toUpperCase() !== "GET") return null;
  if (r.statusCode !== 200 && r.statusCode !== 206) return null;
  const mime = mimeOf(header(r.responseHeaders, "content-type"));
  const ext = pathExtension(r.url);
  const generic = GENERIC_TYPES.has(mime);
  const size = responseSize(r.responseHeaders);
  const hint = (kind: MediaKind): Hint => {
    const h: Hint = { url: r.url, kind };
    const clean = cleanMime(mime);
    if (clean) h.mime = clean;
    if (size !== null && kind === "file") h.size = size;
    return h;
  };
  if (HLS_TYPES.has(mime) || (generic && ext === "m3u8")) return hint("hls");
  if (DASH_TYPES.has(mime) || (generic && ext === "mpd")) return hint("dash");
  const media = mime.startsWith("video/") || mime.startsWith("audio/");
  if (!media && !(generic && FILE_EXTENSIONS.has(ext))) return null;
  if (SEGMENT_TYPES.has(mime) || SEGMENT_EXTENSIONS.has(ext)) return null;
  // A range that does not start at 0 is a piece of a file a player seeks in:
  // its first request (from 0) is the one that counts.
  const range = header(r.responseHeaders, "content-range");
  if (range && !/^\s*bytes\s+0-/i.test(range)) return null;
  if (size !== null && size < MIN_MEDIA_SIZE) return null;
  return hint("file");
}

/** What a tab has shown of media since it moved to its page. */
export interface TabMedia {
  /** The page, without its fragment; null until known. */
  page: string | null;
  hints: Hint[];
}

/** A page's address as the tab record compares it: without the fragment. */
export function pageKey(url: string | null | undefined): string | null {
  if (!url) return null;
  const i = url.indexOf("#");
  return i < 0 ? url : url.slice(0, i);
}

/** Whether a tab that showed `before` and now shows `after` moved to another page. */
export function pageChanged(before: string | null, after: string | null | undefined): boolean {
  const key = pageKey(after);
  return key !== null && key !== before;
}

/**
 * The tab's hints with `hint` added, or null when nothing changes: a link
 * seen before, a file once the tab has a manifest (a segment, most likely),
 * or a full list. A manifest arriving drops the files listed so far for the
 * same reason. Manifests come first, in the order seen.
 */
export function addHint(hints: readonly Hint[], hint: Hint): Hint[] | null {
  if (hints.some((h) => h.url === hint.url)) return null;
  const streams = hints.filter((h) => h.kind !== "file");
  if (hint.kind === "file") {
    if (streams.length > 0 || hints.length >= MAX_TAB_HINTS) return null;
    return [...hints, hint];
  }
  if (streams.length >= MAX_TAB_HINTS) return null;
  return [...streams, hint];
}

/** The toolbar badge for a tab's hints: the count, "" for none. */
export function badgeText(count: number): string {
  if (count <= 0) return "";
  return count > 9 ? "9+" : String(count);
}

/** What the popup finds on the page itself (scanPage). */
export interface PageScan {
  /** `<video>` and `<audio>` elements. */
  elements: number;
  /** The page names a video for sharing (`og:video`, `twitter:player:stream`). */
  declared: boolean;
  /** Direct `http(s)` sources of those elements and of `og:video`. */
  sources: string[];
}

/**
 * Runs in the page (injected once by the popup through activeTab): counts its
 * video and audio elements and reads their direct sources and the page's
 * declared video. Self-contained: no imports reach the page.
 */
export function scanPage(): PageScan {
  const sources: string[] = [];
  const add = (src: string | null | undefined) => {
    if (src && /^https?:/i.test(src) && sources.length < 20 && !sources.includes(src)) {
      sources.push(src);
    }
  };
  const elements = Array.from(document.querySelectorAll("video, audio"));
  for (const el of elements) {
    add((el as HTMLMediaElement).currentSrc || el.getAttribute("src"));
    for (const s of Array.from(el.querySelectorAll("source"))) add(s.src);
  }
  let declared = false;
  for (const meta of Array.from(document.querySelectorAll("meta[property], meta[name]"))) {
    const key = (meta.getAttribute("property") ?? meta.getAttribute("name") ?? "").toLowerCase();
    if (
      key === "og:video" ||
      key === "og:video:url" ||
      key === "og:video:secure_url" ||
      key === "twitter:player:stream"
    ) {
      declared = true;
      add(meta.getAttribute("content"));
    }
  }
  return { elements: elements.length, declared, sources };
}

/** A page scan as the popup takes it from the page: anything else is nothing. */
export function readScan(value: unknown): PageScan {
  const empty: PageScan = { elements: 0, declared: false, sources: [] };
  if (typeof value !== "object" || value === null) return empty;
  const v = value as Record<string, unknown>;
  const elements = typeof v.elements === "number" && v.elements >= 0 ? Math.floor(v.elements) : 0;
  const sources = Array.isArray(v.sources)
    ? v.sources.filter((s): s is string => isFetchable(typeof s === "string" ? s : null))
    : [];
  return { elements, declared: v.declared === true, sources: sources.slice(0, MAX_TAB_HINTS) };
}

/** A direct source the page names, as a hint: its kind from its path. */
export function sourceHint(url: string): Hint | null {
  if (!isFetchable(url)) return null;
  const ext = pathExtension(url);
  if (ext === "m3u8") return { url, kind: "hls" };
  if (ext === "mpd") return { url, kind: "dash" };
  return { url, kind: "file" };
}

/**
 * The hints that go with a `media_page`: the one the user chose, or the
 * tab's and the page's own sources together, once each, at most
 * MAX_TAB_HINTS, the tab's first.
 */
export function hintsToSend(
  tab: readonly Hint[],
  sources: readonly string[],
  chosen: string | null,
): Hint[] {
  if (chosen !== null) {
    const hint = tab.find((h) => h.url === chosen) ?? sourceHint(chosen);
    return hint ? [hint] : [];
  }
  const out: Hint[] = [];
  for (const h of [...tab, ...sources.map(sourceHint)]) {
    if (out.length >= MAX_TAB_HINTS) break;
    if (h && !out.some((x) => x.url === h.url)) out.push(h);
  }
  return out;
}

/** A resolution named in a link (`…/720p/…`, `…_1920x1080.mp4`), if any. */
export function linkResolution(url: string): string | null {
  let path: string;
  try {
    path = decodeURIComponent(new URL(url).pathname);
  } catch {
    return null;
  }
  const p = /(?:^|[^0-9a-z])(\d{3,4})p(?:\d{2})?(?![0-9a-z])/i.exec(path);
  if (p?.[1]) return `${p[1]}p`;
  const wh = /(?:^|[^0-9])\d{3,4}x(\d{3,4})(?![0-9])/i.exec(path);
  return wh?.[1] ? `${wh[1]}p` : null;
}

/** `storage.session`, injected so the logic runs without a browser. */
export interface SessionArea {
  get(key: string): Promise<Record<string, unknown>>;
  set(items: Record<string, unknown>): Promise<void>;
  remove(key: string): Promise<void>;
}

function readTab(value: unknown): TabMedia {
  if (typeof value !== "object" || value === null) return { page: null, hints: [] };
  const v = value as Record<string, unknown>;
  const hints = Array.isArray(v.hints)
    ? v.hints.filter(
        (h): h is Hint =>
          typeof h === "object" &&
          h !== null &&
          isFetchable((h as Hint).url) &&
          ["hls", "dash", "file"].includes((h as Hint).kind),
      )
    : [];
  return { page: typeof v.page === "string" ? v.page : null, hints: hints.slice(0, MAX_TAB_HINTS) };
}

/**
 * Every tab's hints, in `storage.session` (an MV3 service worker forgets its
 * memory when idle; the session area lives as long as the browser and is not
 * written to disk). One change at a time, in order; `onChange` reports a
 * tab's new count for its badge.
 */
export class TabMediaStore {
  readonly #area: SessionArea;
  readonly #onChange: (tabId: number, count: number) => void;
  #queue: Promise<unknown> = Promise.resolve();

  constructor(area: SessionArea, onChange: (tabId: number, count: number) => void) {
    this.#area = area;
    this.#onChange = onChange;
  }

  #key(tabId: number): string {
    return `media:${tabId}`;
  }

  #run<T>(job: () => Promise<T>): Promise<T> {
    const next = this.#queue.then(job, job);
    this.#queue = next.catch(() => undefined);
    return next;
  }

  async #read(tabId: number): Promise<TabMedia> {
    const key = this.#key(tabId);
    return readTab((await this.#area.get(key))[key]);
  }

  /** What tab `tabId` has seen. */
  get(tabId: number): Promise<TabMedia> {
    return this.#run(() => this.#read(tabId));
  }

  /** Adds a hint the tab's page loaded. */
  add(tabId: number, hint: Hint): Promise<void> {
    return this.#run(async () => {
      const tab = await this.#read(tabId);
      const hints = addHint(tab.hints, hint);
      if (!hints) return;
      await this.#area.set({ [this.#key(tabId)]: { page: tab.page, hints } });
      this.#onChange(tabId, hints.length);
    });
  }

  /** The tab shows `page` now: its hints go if it moved to another page. */
  navigated(tabId: number, page: string | null | undefined): Promise<void> {
    return this.#run(async () => {
      const tab = await this.#read(tabId);
      if (!pageChanged(tab.page, page)) return;
      await this.#area.set({ [this.#key(tabId)]: { page: pageKey(page), hints: [] } });
      if (tab.hints.length > 0) this.#onChange(tabId, 0);
    });
  }

  /** Forgets a tab (closed), or its hints (the page is excluded). */
  forget(tabId: number): Promise<void> {
    return this.#run(async () => {
      await this.#area.remove(this.#key(tabId));
      this.#onChange(tabId, 0);
    });
  }
}

/** The popup asks the background what tab `tabId` has seen. */
export interface MediaStateRequest {
  type: "boltget:media-state";
  tabId: number;
}

/** What the popup shows for a tab: whether media is offered there, and the hints. */
export interface MediaState {
  allowed: boolean;
  hints: Hint[];
}

/**
 * The popup asks the background to send tab `tabId`'s page to the app: with
 * `hint`, that one; otherwise every hint and the page's own `sources`.
 */
export interface MediaSendRequest {
  type: "boltget:media-send";
  tabId: number;
  hint: string | null;
  sources: string[];
}

/** How sending a page went. */
export type MediaSendResult = "sent" | "excluded" | "app_too_old" | "failed";

/** A request from the popup, checked: anything else is null. */
export function readMediaRequest(value: unknown): MediaStateRequest | MediaSendRequest | null {
  if (typeof value !== "object" || value === null) return null;
  const v = value as Record<string, unknown>;
  if (typeof v.tabId !== "number" || !Number.isInteger(v.tabId) || v.tabId < 0) return null;
  if (v.type === "boltget:media-state") return { type: v.type, tabId: v.tabId };
  if (v.type !== "boltget:media-send") return null;
  const hint = typeof v.hint === "string" && isFetchable(v.hint) ? v.hint : null;
  const sources = readScan({ sources: v.sources }).sources;
  return { type: v.type, tabId: v.tabId, hint, sources };
}

/** A size as the popup shows it: `512 KiB`, `48 MiB`, `1.4 GiB`. */
export function sizeText(bytes: number): string {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1;
  const text = value.toLocaleString("en", { maximumFractionDigits: digits });
  return `${text} ${units[unit] ?? "B"}`;
}

/** What a hint is, for the popup's catalog. */
export type HintKindKey =
  "media.kind.hls" | "media.kind.dash" | "media.kind.video" | "media.kind.audio";

/** One hint as the popup lists it: what it is, then resolution, host and size. */
export function hintLine(hint: Hint): { kind: HintKindKey; host: string; details: string[] } {
  const kind: HintKindKey =
    hint.kind === "hls"
      ? "media.kind.hls"
      : hint.kind === "dash"
        ? "media.kind.dash"
        : hint.mime?.toLowerCase().startsWith("audio/")
          ? "media.kind.audio"
          : "media.kind.video";
  let host = "";
  try {
    host = new URL(hint.url).hostname;
  } catch {
    // Checked when it was kept.
  }
  const details: string[] = [];
  const res = linkResolution(hint.url);
  if (res) details.push(res);
  if (host) details.push(host);
  if (typeof hint.size === "number") details.push(sizeText(hint.size));
  return { kind, host, details };
}
