// The capture decision (docs/BROWSER-INTEGRATION.md, "Capture rules" and
// "What is never captured"): a pure function of what the browser told us about
// a download and the rules in force, so it can be tested without a browser.
import type { CaptureRules } from "@boltget/native-messaging";
import { onAnySite } from "./rules";

/** The modifier keys held while the link was clicked (content script). */
export type Modifiers = "none" | "bypass" | "force";

/** What the extension knows about a download when it decides. */
export interface CaptureEvent {
  /** The link the download started from (`DownloadItem.url`). */
  url: string;
  /** Where redirects ended (`DownloadItem.finalUrl`). */
  finalUrl?: string | null;
  /** The browser's file name; a full path is fine (Firefox gives one). */
  filename?: string | null;
  /** The response's MIME type. */
  mime?: string | null;
  /** The size in bytes when the browser knows it; 0 or less means unknown. */
  size?: number | null;
  /** `DownloadItem.referrer`. */
  referrer?: string | null;
  /** The tab's URL, when known. */
  pageUrl?: string | null;
  /** From a private (incognito) window. */
  incognito: boolean;
  /** The observed request's method, when the extension saw the request. */
  method?: string | null;
  /** Started by an extension (this one included), not by a page. */
  byExtension?: boolean;
  /** Modifier keys recorded for this link. */
  modifiers?: Modifiers;
  /** The user asked for Boltget explicitly (the link's context menu). */
  forced?: boolean;
}

/** The rules plus the extension's own switches, resolved by the caller. */
export interface CaptureSettings extends CaptureRules {
  /** The popup's switch is on and capture is not paused. */
  active: boolean;
}

/** Why a download goes to Boltget. */
export type CaptureReason = "forced" | "always_site" | "type" | "size";

/** Why a download stays in the browser. */
export type SkipReason =
  /** `blob:`, `data:`, `file:` and anything else the app cannot fetch. */
  | "unsupported_url"
  /** Started by an extension (Boltget's own fallback downloads included). */
  | "extension"
  /** A form submission (`POST`) the app cannot repeat. */
  | "post"
  /** A private window, without "capture in private windows". */
  | "private_window"
  /** The user held the bypass key (Alt). */
  | "bypass"
  /** Capture is off or paused. */
  | "disabled"
  /** The page or the download is on the never-capture list. */
  | "never_site"
  /** Known to be smaller than the threshold. */
  | "small"
  /** Not one of the captured file types. */
  | "type";

/** The outcome of {@link shouldCapture}. */
export type Decision =
  | { capture: true; forced: boolean; reason: CaptureReason }
  | { capture: false; reason: SkipReason };

/** The longest URL the protocol accepts (32 KiB). */
export const MAX_URL = 32 * 1024;

/** Whether the app could fetch this URL: `http`/`https` with a host. */
export function isFetchable(url: string | null | undefined): url is string {
  if (!url || url.length > MAX_URL) return false;
  try {
    const parsed = new URL(url);
    return (parsed.protocol === "http:" || parsed.protocol === "https:") && parsed.hostname !== "";
  } catch {
    return false;
  }
}

/**
 * Extensions that name the script that served a file rather than the file
 * (`download.php?id=4`): they say nothing about the type.
 */
const SCRIPT_EXTENSIONS = new Set([
  "php",
  "asp",
  "aspx",
  "jsp",
  "cgi",
  "pl",
  "py",
  "do",
  "action",
  "htm",
  "html",
  "shtml",
]);

/** The last path segment of a file name or path, either separator. */
export function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? "";
}

/** A name's extension, lowercase, or null when it has none worth using. */
export function extensionOf(name: string | null | undefined): string | null {
  if (!name) return null;
  const base = baseName(name);
  const dot = base.lastIndexOf(".");
  if (dot <= 0 || dot === base.length - 1) return null;
  const ext = base.slice(dot + 1).toLowerCase();
  if (!/^[a-z0-9]{1,16}$/.test(ext) || SCRIPT_EXTENSIONS.has(ext)) return null;
  return ext;
}

/** The extension of a URL's path (query and fragment ignored). */
export function urlExtension(url: string | null | undefined): string | null {
  if (!url) return null;
  try {
    const path = decodeURIComponent(new URL(url).pathname);
    return extensionOf(path);
  } catch {
    return null;
  }
}

/** MIME types that say which listed file type a response is. */
const MIME_EXTENSIONS: Record<string, string> = {
  "application/zip": "zip",
  "application/x-zip-compressed": "zip",
  "application/vnd.rar": "rar",
  "application/x-rar-compressed": "rar",
  "application/x-7z-compressed": "7z",
  "application/x-tar": "tar",
  "application/gzip": "gz",
  "application/x-gzip": "gz",
  "application/x-bzip2": "bz2",
  "application/x-xz": "xz",
  "application/zstd": "zst",
  "application/x-iso9660-image": "iso",
  "application/x-msdownload": "exe",
  "application/x-msdos-program": "exe",
  "application/vnd.microsoft.portable-executable": "exe",
  "application/x-msi": "msi",
  "application/x-ms-installer": "msi",
  "application/msix": "msix",
  "application/vnd.ms-appx": "appx",
  "application/x-apple-diskimage": "dmg",
  "application/vnd.debian.binary-package": "deb",
  "application/x-debian-package": "deb",
  "application/x-rpm": "rpm",
  "application/x-redhat-package-manager": "rpm",
  "application/vnd.android.package-archive": "apk",
  "video/mp4": "mp4",
  "video/x-matroska": "mkv",
  "video/webm": "webm",
  "video/quicktime": "mov",
  "video/x-msvideo": "avi",
  "video/x-m4v": "m4v",
  "audio/mpeg": "mp3",
  "audio/flac": "flac",
  "audio/x-flac": "flac",
  "audio/wav": "wav",
  "audio/x-wav": "wav",
  "audio/vnd.wave": "wav",
  "audio/mp4": "m4a",
  "audio/x-m4a": "m4a",
  "audio/ogg": "ogg",
  "audio/opus": "opus",
};

/** MIME types that only say "some bytes". */
const GENERIC_MIME = new Set([
  "",
  "application/octet-stream",
  "binary/octet-stream",
  "application/binary",
  "application/force-download",
  "application/x-download",
  "application/download",
  "application/unknown",
]);

function essence(mime: string | null | undefined): string {
  return (mime ?? "").split(";")[0]?.trim().toLowerCase() ?? "";
}

/** Whether a MIME type is generic binary (or absent). */
export function isGenericMime(mime: string | null | undefined): boolean {
  return GENERIC_MIME.has(essence(mime));
}

/** What the name, the URL and the MIME type say about the file's type. */
export type TypeMatch = "match" | "no_match" | "unknown";

/**
 * Matches the download against the listed types using, in order, the file
 * name, the URL's path and the MIME type: the first that names a type
 * decides.
 */
export function matchType(
  event: Pick<CaptureEvent, "filename" | "url" | "finalUrl" | "mime">,
  extensions: string[],
): TypeMatch {
  const ext =
    extensionOf(event.filename) ??
    urlExtension(event.finalUrl) ??
    urlExtension(event.url) ??
    MIME_EXTENSIONS[essence(event.mime)] ??
    null;
  if (ext !== null) return extensions.includes(ext) ? "match" : "no_match";
  return isGenericMime(event.mime) ? "unknown" : "no_match";
}

/** Decides whether a download goes to Boltget or stays in the browser. */
export function shouldCapture(event: CaptureEvent, settings: CaptureSettings): Decision {
  if (!isFetchable(event.url)) return { capture: false, reason: "unsupported_url" };
  if (event.finalUrl && event.finalUrl !== event.url && !isFetchable(event.finalUrl)) {
    return { capture: false, reason: "unsupported_url" };
  }
  if (event.byExtension) return { capture: false, reason: "extension" };
  if (event.method && event.method.toUpperCase() !== "GET") {
    return { capture: false, reason: "post" };
  }

  const modifiers = event.modifiers ?? "none";
  if (modifiers === "bypass") return { capture: false, reason: "bypass" };

  // An explicit request (the link's menu, Alt+Shift) is consent, also in a
  // private window: the message tells the app it came from one.
  const forced = event.forced === true || modifiers === "force";
  if (forced) return { capture: true, forced: true, reason: "forced" };

  if (event.incognito && !settings.private_windows) {
    return { capture: false, reason: "private_window" };
  }
  if (!settings.enabled || !settings.active) return { capture: false, reason: "disabled" };

  const pages = [event.url, event.finalUrl, event.referrer, event.pageUrl];
  if (onAnySite(pages, settings.never_sites)) return { capture: false, reason: "never_site" };
  if (onAnySite(pages, settings.always_sites)) {
    return { capture: true, forced: false, reason: "always_site" };
  }

  const size = typeof event.size === "number" && event.size > 0 ? event.size : null;
  const type = matchType(event, settings.extensions);

  if (type === "match") {
    if (size !== null && size < settings.min_size) return { capture: false, reason: "small" };
    return { capture: true, forced: false, reason: "type" };
  }
  // Generic bytes with no telling name: big enough is reason enough.
  if (type === "unknown" && size !== null) {
    if (size >= settings.min_size) return { capture: true, forced: false, reason: "size" };
    return { capture: false, reason: "small" };
  }
  return { capture: false, reason: "type" };
}
