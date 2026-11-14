// Settings form logic (docs/APP-DESIGN.md, "Settings as built"): defaults,
// the checks a field makes before its value is sent (the backend checks the
// same rules in `src-tauri/src/settings.rs` and refuses anything else), and
// where a new download goes. Problems are whole sentences from the catalog,
// shown under the field.
import { t } from "@boltget/ui/messages";
import type { AppSettings, Category as CategorySetting, CategoryId } from "../bindings";
import { DEFAULT_CATEGORIES, extensionOf } from "../list/category";

export const MIB = 1024 * 1024;
export const MAX_CONNECTIONS = 32;

/** The defaults, as the backend has them (used until the engine answers). */
export const DEFAULT_SETTINGS: AppSettings = {
  speedLimit: null,
  maxConnections: 16,
  downloadFolder: null,
  categories: DEFAULT_CATEGORIES,
  proxy: { mode: "none", url: "", bypass: "" },
  theme: "system",
  notifications: true,
  closeToTray: true,
  startWithWindows: false,
  clipboard: false,
  browserAsk: false,
};

export type Check<T> = { ok: true; value: T } | { ok: false; problem: string };

const ok = <T>(value: T): Check<T> => ({ ok: true, value });
const bad = (problem: string): Check<never> => ({ ok: false, problem });

/** Connections per download: a whole number from 1 to 32. */
export function parseConnections(text: string): Check<number> {
  const s = text.trim();
  const n = Number(s);
  return /^\d+$/.test(s) && n >= 1 && n <= MAX_CONNECTIONS
    ? ok(n)
    : bad(t("settings.problem.connections"));
}

/** A speed limit in MiB/s (a comma works as the decimal point); empty is none. */
export function parseSpeedLimit(text: string): Check<number | null> {
  const s = text.trim().replace(",", ".");
  if (!s) return ok(null);
  const value = Number(s);
  const bytes = Math.round(value * MIB);
  return /^\d*\.?\d+$/.test(s) && bytes > 0 ? ok(bytes) : bad(t("settings.problem.limit"));
}

/** A speed limit as the field shows it: MiB/s without trailing zeros. */
export function formatSpeedLimit(bytes: number | null): string {
  if (!bytes) return "";
  return String(Math.round((bytes / MIB) * 100) / 100);
}

/** An absolute Windows path: `D:\…`, `D:/…` or a share (`\\server\share`). */
export function isAbsolutePath(path: string): boolean {
  return /^[a-zA-Z]:[\\/]/.test(path) || /^\\\\[^\\]+\\[^\\]+/.test(path);
}

/** A folder typed or picked: a full path, or empty for the default. */
export function parseFolder(text: string): Check<string | null> {
  const s = text.trim();
  if (!s) return ok(null);
  return isAbsolutePath(s) ? ok(s) : bad(t("settings.problem.folder_relative"));
}

const EXTENSION = /^[a-z0-9_+-]{1,16}$/;
const MAX_EXTENSIONS = 200;

/**
 * A category's file types as typed: separated by spaces, commas or
 * semicolons, with or without `.` or `*.` in front. An extension another
 * category has is refused, naming that category.
 */
export function parseExtensions(
  text: string,
  id: CategoryId,
  categories: readonly CategorySetting[],
  categoryName: (id: CategoryId) => string,
): Check<string[]> {
  const out: string[] = [];
  for (const raw of text.split(/[\s,;]+/).filter(Boolean)) {
    const ext = raw.replace(/^\*/, "").replace(/^\.+/, "").toLowerCase();
    if (!EXTENSION.test(ext)) return bad(t("settings.problem.extension"));
    if (!out.includes(ext)) out.push(ext);
  }
  if (out.length > MAX_EXTENSIONS) return bad(t("settings.problem.extensions_many"));
  for (const ext of out) {
    const other = categories.find((c) => c.id !== id && c.extensions.includes(ext));
    if (other) {
      return bad(
        t("settings.problem.extension_in", { extension: ext, category: categoryName(other.id) }),
      );
    }
  }
  return ok(out);
}

/**
 * A manual proxy's address, as the engine's `Proxy::parse` takes it:
 * `host:port`, or `http://`, `https://`, `socks5://` or `socks5h://` in
 * front. A user name or password is refused (Boltget keeps no secrets).
 */
export function parseProxyUrl(text: string): Check<string> {
  const s = text.trim();
  if (!s) return bad(t("settings.problem.proxy_empty"));
  const withScheme = s.includes("://") ? s : `http://${s}`;
  let url: URL;
  try {
    url = new URL(withScheme);
  } catch {
    return bad(t("settings.problem.proxy"));
  }
  if (!["http:", "https:", "socks5:", "socks5h:"].includes(url.protocol) || !url.hostname) {
    return bad(t("settings.problem.proxy"));
  }
  if (url.username || url.password) return bad(t("settings.problem.proxy_credentials"));
  return ok(s);
}

/**
 * Where a new download goes when the user does not choose: its category's
 * folder (Other for an unknown type), else the default download folder,
 * else `fallback` (Windows' Downloads folder).
 */
export function folderFor(
  name: string | null,
  settings: Pick<AppSettings, "categories" | "downloadFolder">,
  fallback: string | null,
): string | null {
  const ext = name ? extensionOf(name) : null;
  const category =
    (ext && settings.categories.find((c) => c.id !== "other" && c.extensions.includes(ext))) ||
    settings.categories.find((c) => c.id === "other");
  return category?.folder || settings.downloadFolder || fallback;
}
