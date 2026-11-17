// A playlist in the media sheet (docs/VIDEO.md, "Playlists: as built"):
// which entries are chosen, how a click, Shift and the keyboard change that,
// the presets offered for all of them, and the request that starts them.
// Pure functions over the backend's values, tested without a DOM.
import { t } from "@boltget/ui/messages";
import type { MediaContainer, MediaEntry, MediaPlaylistRequest, MediaSummary } from "../bindings";
import { mediaLength } from "./picker";

/** The presets of a playlist, in the picker's order. */
export const PLAYLIST_PRESETS = ["best", "1080p", "720p", "small", "audio"] as const;
export type PlaylistPreset = (typeof PLAYLIST_PRESETS)[number];

const HEIGHTS: Partial<Record<PlaylistPreset, number>> = { "1080p": 1080, "720p": 720, small: 480 };

/**
 * What a playlist preset gives, without sizes: entries are not probed one
 * by one, so the preset is a format expression whose own fallbacks choose
 * for each entry.
 */
export function playlistPresetDetails(preset: PlaylistPreset, container: MediaContainer): string[] {
  const parts: string[] = [];
  const height = HEIGHTS[preset];
  if (preset === "best") parts.push(t("media.playlist.best"));
  else if (preset === "audio") parts.push(t("media.playlist.audio"));
  else if (height) parts.push(t("media.playlist.up_to", { height }));
  if (container === "mp4") parts.push(preset === "audio" ? "M4A" : "MP4");
  else parts.push(t("media.playlist.original"));
  return parts;
}

/** The entries that can be chosen (not private or deleted), by place. */
export function choosable(entries: readonly MediaEntry[]): number[] {
  return entries.filter((e) => e.available && e.url).map((e) => e.index);
}

/** Every entry that can be chosen. */
export function selectAll(entries: readonly MediaEntry[]): Set<number> {
  return new Set(choosable(entries));
}

/** One entry chosen or not, as a click or Space does. */
export function toggle(
  selected: ReadonlySet<number>,
  entries: readonly MediaEntry[],
  index: number,
): Set<number> {
  const next = new Set(selected);
  if (!choosable(entries).includes(index)) return next;
  if (next.has(index)) next.delete(index);
  else next.add(index);
  return next;
}

/**
 * Shift with a click or Space: every entry from the anchor to `index` (in
 * list order, both included) takes the anchor's state, chosen or not, as
 * check boxes in a file manager do. Unavailable entries are skipped.
 */
export function selectRange(
  selected: ReadonlySet<number>,
  entries: readonly MediaEntry[],
  anchor: number | null,
  index: number,
): Set<number> {
  const next = new Set(selected);
  const order = entries.map((e) => e.index);
  const to = order.indexOf(index);
  if (to < 0) return next;
  const from = anchor === null ? to : order.indexOf(anchor);
  const start = Math.min(from < 0 ? to : from, to);
  const end = Math.max(from < 0 ? to : from, to);
  const value = anchor === null || from < 0 ? true : selected.has(anchor);
  for (const e of entries.slice(start, end + 1)) {
    if (!e.available || !e.url) continue;
    if (value) next.add(e.index);
    else next.delete(e.index);
  }
  return next;
}

/** The entry `by` places from `index` (clamped), for the arrow keys. */
export function moveActive(
  entries: readonly MediaEntry[],
  index: number | null,
  by: number,
): number | null {
  if (entries.length === 0) return null;
  const at = index === null ? -1 : entries.findIndex((e) => e.index === index);
  const next = at < 0 ? (by > 0 ? 0 : entries.length - 1) : at + by;
  return entries[Math.max(0, Math.min(entries.length - 1, next))]?.index ?? null;
}

/** The entries after a "Load more", without places listed twice. */
export function mergeEntries(
  existing: readonly MediaEntry[],
  more: readonly MediaEntry[],
): MediaEntry[] {
  const seen = new Set(existing.map((e) => e.index));
  return [...existing, ...more.filter((e) => !seen.has(e.index))];
}

/** "12 of 58 selected" (of the entries that can be chosen). */
export function selectionText(
  selected: ReadonlySet<number>,
  entries: readonly MediaEntry[],
): string {
  const total = choosable(entries).length;
  const count = choosable(entries).filter((i) => selected.has(i)).length;
  return t("media.playlist.selected", { count, total });
}

/** An entry's line for screen readers and tooltips: "7. Title, 3:32". */
export function entryLabel(entry: MediaEntry): string {
  const title = entry.title?.trim() || t("media.playlist.untitled");
  const parts = [`${entry.index}. ${title}`];
  if (entry.duration !== null) parts.push(mediaLength(entry.duration));
  if (!entry.available) parts.push(t("media.playlist.unavailable"));
  return parts.join(", ");
}

/**
 * Subtitle languages a playlist offers: entries are not probed, so these
 * are common languages, each "if available" (the entry's own subtitles,
 * else automatic captions). Named in English by the browser's own list.
 */
export const PLAYLIST_SUBTITLES = [
  "en",
  "es",
  "fr",
  "de",
  "pt",
  "it",
  "nl",
  "pl",
  "tr",
  "ru",
  "uk",
  "ar",
  "hi",
  "ja",
  "ko",
  "zh-Hans",
] as const;

const languages =
  typeof Intl.DisplayNames === "function"
    ? new Intl.DisplayNames(["en"], { type: "language" })
    : null;

/** The subtitle choices of a playlist: value is the language code. */
export function playlistSubtitleOptions(): { value: string; label: string }[] {
  return PLAYLIST_SUBTITLES.map((code) => {
    let name: string = code;
    try {
      name = languages?.of(code) ?? code;
    } catch {
      // An unknown code keeps its code as its name.
    }
    return { value: code, label: t("media.playlist.subtitles.language", { name }) };
  });
}

/** The request that starts the chosen entries, in the playlist's order. */
export function playlistRequest(
  media: MediaSummary,
  entries: readonly MediaEntry[],
  selected: ReadonlySet<number>,
  options: {
    preset: PlaylistPreset;
    container: MediaContainer;
    subtitle: string | null;
    embed: boolean;
    page: number | null;
  },
): MediaPlaylistRequest {
  const chosen = entries
    .filter((e) => selected.has(e.index) && e.available && e.url)
    .sort((a, b) => a.index - b.index);
  const audio = options.preset === "audio";
  return {
    url: media.url,
    title: media.title,
    uploader: media.uploader,
    id: media.playlist?.id ?? null,
    preset: options.preset,
    container: options.container,
    subtitles: options.subtitle
      ? { languages: [options.subtitle], automatic: true, embed: options.embed && !audio }
      : null,
    entryCount: Math.max(media.entryCount, ...entries.map((e) => e.index)),
    entries: chosen.map((e) => ({ index: e.index, url: e.url ?? "", title: e.title })),
    page: options.page,
  };
}
