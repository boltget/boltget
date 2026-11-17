// Development only: clipboard monitoring for the UI alone in a browser
// (`pnpm --filter @boltget/desktop dev`), standing in for the native
// listener (src-tauri/src/clipboard.rs). native.ts loads it only when
// `import.meta.env.DEV` and outside Tauri.
//
// With "Offer copied links" on, the clipboard is read when the page gains
// focus (copy a link in another tab or app, then come back; the browser asks
// once whether the page may read it), and `__boltget.copy(text)` in the
// devtools console stands in for another app copying `text`. What is offered
// follows the app's rules: a link to a video page of a site in
// `video_sites.json` (the same file the app reads), else links that name a
// file of a known category; each link once.
import sites from "../../../src-tauri/src/clipboard/video_sites.json";
import type { AppSettings, ClipboardOffer } from "../bindings";

interface Site {
  site: string;
  audio?: boolean;
  hosts: string[];
  paths: string[];
  except?: string[];
  examples: string[];
}

/** The list the app ships, with the examples its tests check. */
export const VIDEO_SITES: { sites: Site[]; notPages: string[] } = sites;

/** As the app's `video_page`: the site of a video page link, if it is one. */
export function videoPage(link: string): { site: string; audio: boolean } | null {
  let url: URL;
  try {
    url = new URL(link);
  } catch {
    return null;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return null;
  const host = url.hostname.replace(/\.$/, "").toLowerCase();
  const segments = url.pathname
    .split("/")
    .filter(Boolean)
    .map((s) => {
      try {
        return decodeURIComponent(s).toLowerCase();
      } catch {
        return s.toLowerCase();
      }
    });
  const pathMatches = (pattern: string) => matchesPath(pattern, segments, url.searchParams);
  const site = VIDEO_SITES.sites.find(
    (s) =>
      s.hosts.some((h) => matchesHost(h, host)) &&
      s.paths.some(pathMatches) &&
      !(s.except ?? []).some(pathMatches),
  );
  return site ? { site: site.site, audio: site.audio ?? false } : null;
}

function matchesHost(pattern: string, host: string): boolean {
  if (!pattern.startsWith("*.")) return host === pattern;
  const domain = pattern.slice(2);
  return host === domain || host.endsWith(`.${domain}`);
}

function matchesPath(pattern: string, segments: string[], query: URLSearchParams): boolean {
  const [path = "", key = ""] = pattern.split("?");
  const wanted = path.split("/").filter(Boolean);
  return (
    wanted.length > 0 &&
    wanted.length <= segments.length &&
    wanted.every((w, i) => matchesSegment(w.toLowerCase(), segments[i] ?? "")) &&
    (!key || !!query.get(key))
  );
}

function matchesSegment(pattern: string, segment: string): boolean {
  if (pattern === "*") return true;
  if (pattern === "{n}") return /^\d+$/.test(segment);
  const star = pattern.indexOf("*");
  if (star < 0) return pattern === segment;
  const head = pattern.slice(0, star);
  const tail = pattern.slice(star + 1);
  return (
    segment.length > head.length + tail.length && segment.startsWith(head) && segment.endsWith(tail)
  );
}

/** As the app's `offer_for`, with a simpler idea of a file (its name only). */
export function offerFor(
  text: string,
  settings: Pick<AppSettings, "categories" | "clipboardVideo">,
): ClipboardOffer | null {
  const tokens = text.split(/\s+/).filter(Boolean);
  if (tokens.length === 0 || tokens.length > 50) return null;
  const only = tokens.length === 1 ? tokens[0] : undefined;
  const page = settings.clipboardVideo && only ? videoPage(only) : null;
  if (only && page) return { kind: "page", url: only, ...page };
  const known = new Set(settings.categories.flatMap((c) => c.extensions));
  const links: string[] = [];
  for (const token of tokens) {
    if (!/^https?:\/\//i.test(token)) return null;
    let name: string;
    try {
      name = decodeURIComponent(new URL(token).pathname.split("/").pop() ?? "");
    } catch {
      return null;
    }
    const ext = /\.([^.]+)$/.exec(name)?.[1]?.toLowerCase();
    if (ext && name.length > ext.length + 1 && known.has(ext) && !links.includes(token)) {
      links.push(token);
    }
  }
  return links.length ? { kind: "files", links: links.join("\n") } : null;
}

let deliver: ((text: string) => void) | null = null;

/** Another app copied `text` (the devtools' `__boltget.copy`). */
export function simulateCopy(text: string): void {
  deliver?.(text);
}

/**
 * Watches the clipboard while `settings().clipboard` is on, as the app's
 * listener does, and calls `handler` with each new offer.
 */
export function watchClipboard(
  settings: () => AppSettings,
  handler: (offer: ClipboardOffer) => void,
): () => void {
  const offered: string[] = [];
  deliver = (text) => {
    const s = settings();
    if (!s.clipboard) return;
    const offer = offerFor(text, s);
    if (!offer) return;
    const links = (offer.kind === "page" ? [offer.url] : offer.links.split("\n")).filter(
      (l) => !offered.includes(l),
    );
    if (links.length === 0) return;
    offered.push(...links);
    handler(offer.kind === "page" ? offer : { kind: "files", links: links.join("\n") });
  };
  const onFocus = () => {
    if (!settings().clipboard || !navigator.clipboard?.readText) return;
    navigator.clipboard.readText().then(
      (text) => deliver?.(text),
      () => {},
    );
  };
  // (No window in the unit tests.)
  globalThis.addEventListener?.("focus", onFocus);
  return () => {
    globalThis.removeEventListener?.("focus", onFocus);
    deliver = null;
  };
}
