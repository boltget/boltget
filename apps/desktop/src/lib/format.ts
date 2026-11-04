// Human units for the list and the details pane (DESIGN-PRINCIPLES.md,
// "Information density done right"): binary units with clear labels,
// sensible rounding, time left in words when long and precise when short.
// Every unit and phrase comes from the string catalog.
import { t } from "@boltget/ui/messages";

const KIB = 1024;
const UNIT_KEYS = ["unit.b", "unit.kib", "unit.mib", "unit.gib", "unit.tib"] as const;
const SPEED_KEYS = ["speed.b", "speed.kib", "speed.mib", "speed.gib", "speed.tib"] as const;

const formats = new Map<string, Intl.NumberFormat>();
function number(value: number, digits: number, fixed: boolean): string {
  const key = `${digits}${fixed ? "f" : ""}`;
  let format = formats.get(key);
  if (!format) {
    format = new Intl.NumberFormat("en", {
      maximumFractionDigits: digits,
      minimumFractionDigits: fixed ? digits : 0,
    });
    formats.set(key, format);
  }
  return format.format(value);
}

/** The binary unit (0 = B, 1 = KiB, …) that shows `bytes` best. */
export function unitOf(bytes: number): number {
  let unit = 0;
  let value = Math.abs(bytes);
  while (value >= 1000 && unit < UNIT_KEYS.length - 1) {
    value /= KIB;
    unit += 1;
  }
  return unit;
}

/**
 * `bytes` in `unit`, rounded: 2 decimals under 10, 1 under 100, then none.
 * `fixed` keeps trailing zeros, for numbers that change live (`0.40`), so
 * their width does not jump; settled sizes drop them (`7 MiB`).
 */
export function scaled(bytes: number, unit: number, fixed = false): string {
  const value = bytes / KIB ** unit;
  if (unit === 0) return number(value, 0, false);
  const abs = Math.abs(value);
  return number(value, abs < 10 ? 2 : abs < 100 ? 1 : 0, fixed);
}

/** `420 MiB`, `1.25 GiB`, `512 B`. */
export function formatBytes(bytes: number): string {
  const unit = unitOf(bytes);
  return t(UNIT_KEYS[unit] ?? "unit.b", { value: scaled(bytes, unit) });
}

/** `212 / 420 MiB`: both numbers in the total's unit, so they line up. */
export function formatProgress(downloaded: number, total: number | null): string {
  if (total === null) return formatBytes(downloaded);
  const unit = unitOf(total);
  return t("size.progress", {
    done: scaled(Math.min(downloaded, total), unit, true),
    total: t(UNIT_KEYS[unit] ?? "unit.b", { value: scaled(total, unit) }),
  });
}

/** A formatted message cut around its number, so the number can be styled. */
export interface Parts {
  before: string;
  value: string;
  after: string;
}

const MARK = "";

function cut(text: string, value: string): Parts {
  const [before = "", after = ""] = text.split(MARK);
  return { before, value, after };
}

/** `420` + ` MiB`. */
export function bytesParts(bytes: number): Parts {
  const unit = unitOf(bytes);
  const value = scaled(bytes, unit);
  return cut(t(UNIT_KEYS[unit] ?? "unit.b", { value: MARK }), value);
}

/** `48.2` + ` MiB/s`. */
export function speedParts(bps: number): Parts {
  const unit = unitOf(bps);
  const value = scaled(bps, unit, true);
  return cut(t(SPEED_KEYS[unit] ?? "speed.b", { value: MARK }), value);
}

/** `212` + ` / 420 MiB`. */
export function progressParts(downloaded: number, total: number | null): Parts {
  if (total === null) return bytesParts(downloaded);
  const unit = unitOf(total);
  const value = scaled(Math.min(downloaded, total), unit, true);
  const totalText = t(UNIT_KEYS[unit] ?? "unit.b", { value: scaled(total, unit) });
  return cut(t("size.progress", { done: MARK, total: totalText }), value);
}

/** A duration for the large stats: `12 s`, `4 min`, `2 h 10 min`. */
export function formatDuration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return t("duration.seconds", { count: Math.max(1, s) });
  const minutes = Math.round(s / 60);
  if (minutes < 60) return t("duration.minutes", { count: minutes });
  return t("duration.hours", { hours: Math.floor(minutes / 60), minutes: minutes % 60 });
}

/** `48.2 MiB/s`. */
export function formatSpeed(bps: number): string {
  const unit = unitOf(bps);
  return t(SPEED_KEYS[unit] ?? "speed.b", { value: scaled(bps, unit, true) });
}

/** Seconds left, or null when unknown (no total, or nothing flowing). */
export function secondsLeft(downloaded: number, total: number | null, bps: number): number | null {
  if (total === null || bps <= 0) return null;
  return Math.max(0, (total - downloaded) / bps);
}

/**
 * The time left to show: the new estimate only when it moved by more than
 * 10% from the one shown (APP-DESIGN #27), so the words do not flicker.
 */
export function stableEta(shown: number | null, next: number | null): number | null {
  if (next === null || shown === null) return next;
  if (shown === 0) return next;
  return Math.abs(next - shown) / shown > 0.1 ? next : shown;
}

/** `8 s left`, `about 3 min left`, `about 2 h 10 min left`. */
export function formatEta(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return t("eta.seconds", { count: Math.max(1, s) });
  const minutes = Math.round(s / 60);
  if (minutes < 60) return t("eta.minutes", { count: minutes });
  if (s >= 86_400) return t("eta.days");
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 || hours >= 10
    ? t("eta.hours", { count: hours })
    : t("eta.hours_minutes", { hours, minutes: rest });
}

/** The host of a link, without `www.`. */
export function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return url;
  }
}

/** The name to show: the probed one, else the link's last path segment. */
export function displayName(fileName: string | null, url: string): string {
  if (fileName) return fileName;
  try {
    const parsed = new URL(url);
    const last = parsed.pathname.split("/").filter(Boolean).pop();
    if (last) return decodeURIComponent(last);
    return parsed.hostname;
  } catch {
    return url;
  }
}

const times = new Intl.DateTimeFormat("en", { hour: "numeric", minute: "2-digit" });
const days = new Intl.DateTimeFormat("en", { month: "short", day: "numeric" });
const daysWithYear = new Intl.DateTimeFormat("en", {
  year: "numeric",
  month: "short",
  day: "numeric",
});

/** When something finished: the time today, else the date. */
export function formatWhen(ms: number, now: number = Date.now()): string {
  const date = new Date(ms);
  const today = new Date(now);
  if (date.toDateString() === today.toDateString()) return times.format(date);
  if (date.getFullYear() === today.getFullYear()) return days.format(date);
  return daysWithYear.format(date);
}

/** `48 %` style percentages as whole numbers, never 100 before the end. */
export function percent(downloaded: number, total: number | null): number | null {
  if (total === null || total <= 0) return null;
  const p = Math.floor((downloaded / total) * 100);
  return Math.min(downloaded >= total ? 100 : 99, Math.max(0, p));
}

/**
 * A path or link in pieces that each end at a separator (`\`, `/`), so it
 * can wrap between folders instead of in the middle of a name.
 */
export function pathPieces(path: string): string[] {
  const pieces = path.match(/[^\\/]*(?:[\\/]+|$)/g)?.filter(Boolean) ?? [path];
  // A link's scheme stays with its host, which may wrap after its dots:
  // "https://cdn.", "example.org/", never "https://" alone.
  if (pieces.length > 1 && pieces[0]?.endsWith("://")) {
    const host = pieces[1]?.match(/[^.]*(?:\.+|$)/g)?.filter(Boolean) ?? [];
    pieces.splice(0, 2, `${pieces[0]}${host[0] ?? ""}`, ...host.slice(1));
  }
  return pieces;
}

/** A folder's last name and the path above it, for compact display. */
export function splitFolder(path: string): { parent: string; leaf: string } {
  const trimmed = path.replace(/[\\/]+$/, "");
  const cut = Math.max(trimmed.lastIndexOf("\\"), trimmed.lastIndexOf("/"));
  if (cut < 0) return { parent: "", leaf: trimmed || path };
  return { parent: trimmed.slice(0, cut + 1), leaf: trimmed.slice(cut + 1) || trimmed };
}
