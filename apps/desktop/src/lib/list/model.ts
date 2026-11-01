// What the main list shows, in what order (APP-DESIGN #2 to #4): the view
// and search filter it, "All" is grouped (Active, then Queued and paused,
// then history by day), and rows never move on their own while the user is
// interacting with the list. Sorting happens only on request.
//
// Pure functions over plain values, so they are tested without a DOM; the
// components feed them the store's reactive `Download`s.
import type { DownloadId, DownloadState } from "../bindings";
import { categoryOf, type Category } from "./category";

export type View = "all" | "active" | "completed" | "scheduled" | Category;
export type Sort = "default" | "name" | "size" | "added";
export type Section = "active" | "waiting" | "today" | "yesterday" | "week" | "earlier";

/** The values of a download the list orders and filters by. */
export interface ListItem {
  id: DownloadId;
  state: DownloadState;
  name: string;
  url: string;
  total: number | null;
  createdAt: number;
  completedAt: number | null;
}

export type Entry =
  | { kind: "section"; key: string; section: Section; count: number }
  | { kind: "row"; key: string; id: DownloadId; index: number };

const RUNNING: ReadonlySet<DownloadState> = new Set([
  "probing",
  "downloading",
  "retrying",
  "verifying",
]);

/** A task works on it: it has live progress. */
export function isRunning(state: DownloadState): boolean {
  return RUNNING.has(state);
}

/** Running, or waiting its turn: what the Active view shows. */
export function isActive(state: DownloadState): boolean {
  return state === "queued" || isRunning(state);
}

/** Stopped and wants the user: failed, corrupt or a link to refresh. */
export function needsAttention(state: DownloadState): boolean {
  return state === "failed" || state === "corrupt" || state === "needs_new_link";
}

const DAY = 86_400_000;

function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/** The group a download belongs to in a grouped view. */
export function sectionOf(item: ListItem, now: number): Section {
  if (isActive(item.state)) return "active";
  if (item.state !== "completed") return "waiting";
  const at = item.completedAt ?? item.createdAt;
  const today = startOfDay(now);
  if (at >= today) return "today";
  if (at >= today - DAY) return "yesterday";
  if (at >= today - 6 * DAY) return "week";
  return "earlier";
}

export const SECTION_ORDER: readonly Section[] = [
  "active",
  "waiting",
  "today",
  "yesterday",
  "week",
  "earlier",
];

/** Whether a download belongs in a view. Scheduled has no backing yet. */
export function inView(item: ListItem, view: View): boolean {
  switch (view) {
    case "all":
      return true;
    case "active":
      return isActive(item.state);
    case "completed":
      return item.state === "completed";
    case "scheduled":
      return false;
    default:
      return categoryOf(item.name) === view;
  }
}

/** Case-insensitive search over the name and the link. */
export function matchesQuery(item: ListItem, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return item.name.toLowerCase().includes(q) || item.url.toLowerCase().includes(q);
}

const collator = new Intl.Collator("en", { numeric: true, sensitivity: "base" });

function compare(sort: Sort): (a: ListItem, b: ListItem) => number {
  switch (sort) {
    case "name":
      return (a, b) => collator.compare(a.name, b.name) || a.id - b.id;
    case "size":
      return (a, b) => (b.total ?? -1) - (a.total ?? -1) || a.id - b.id;
    case "added":
      return (a, b) => b.createdAt - a.createdAt || b.id - a.id;
    default:
      // Newest first. History by when it finished; attention first in waiting.
      return (a, b) =>
        Number(needsAttention(b.state)) - Number(needsAttention(a.state)) ||
        (b.completedAt ?? b.createdAt) - (a.completedAt ?? a.createdAt) ||
        b.id - a.id;
  }
}

export interface BuildOptions {
  view: View;
  query: string;
  sort: Sort;
  now: number;
  /**
   * Sections held while the user interacts with the list: a download keeps
   * the group it had, so nothing moves under the pointer or the keyboard.
   */
  held?: ReadonlyMap<DownloadId, Section>;
}

export interface Layout {
  entries: Entry[];
  /** Row ids in display order, for selection and keyboard movement. */
  order: DownloadId[];
  /** The section each shown download is in (to hold them later). */
  sections: Map<DownloadId, Section>;
}

/** The entries of the list: section headers and rows, in display order. */
export function buildLayout(items: readonly ListItem[], options: BuildOptions): Layout {
  const shown = items.filter(
    (item) => inView(item, options.view) && matchesQuery(item, options.query),
  );
  const sections = new Map<DownloadId, Section>();
  const entries: Entry[] = [];
  const order: DownloadId[] = [];

  if (options.sort !== "default") {
    shown.sort(compare(options.sort));
    for (const item of shown) {
      sections.set(item.id, sectionOf(item, options.now));
      entries.push({ kind: "row", key: `r${item.id}`, id: item.id, index: order.length });
      order.push(item.id);
    }
    return { entries, order, sections };
  }

  const groups = new Map<Section, ListItem[]>(SECTION_ORDER.map((s) => [s, []]));
  for (const item of shown) {
    const section = options.held?.get(item.id) ?? sectionOf(item, options.now);
    sections.set(item.id, section);
    groups.get(section)?.push(item);
  }
  const byNewest = compare("default");
  const byAdded = compare("added");
  for (const section of SECTION_ORDER) {
    const group = groups.get(section) ?? [];
    if (group.length === 0) continue;
    // Running downloads keep the order they were added in, so a row does not
    // jump when another one starts or finishes.
    group.sort(section === "active" ? byAdded : byNewest);
    entries.push({ kind: "section", key: `s${section}`, section, count: group.length });
    for (const item of group) {
      entries.push({ kind: "row", key: `r${item.id}`, id: item.id, index: order.length });
      order.push(item.id);
    }
  }
  return { entries, order, sections };
}

/** How many downloads each view holds, for the sidebar. */
export function viewCounts(
  items: readonly { state: DownloadState }[],
): Record<"all" | "active" | "completed" | "scheduled", number> {
  let active = 0;
  let completed = 0;
  for (const item of items) {
    if (isActive(item.state)) active += 1;
    else if (item.state === "completed") completed += 1;
  }
  return { all: items.length, active, completed, scheduled: 0 };
}

// Virtual scrolling ---------------------------------------------------------

/** Top offsets of each entry, plus the total height as the last value. */
export function offsets(entries: readonly Entry[], rowHeight: number, sectionHeight: number) {
  const tops = new Float64Array(entries.length + 1);
  let y = 0;
  for (let i = 0; i < entries.length; i += 1) {
    tops[i] = y;
    y += entries[i]?.kind === "section" ? sectionHeight : rowHeight;
  }
  tops[entries.length] = y;
  return tops;
}

/** The first entry whose bottom is below `y` (binary search). */
export function entryAt(tops: Float64Array, y: number): number {
  let lo = 0;
  let hi = tops.length - 2;
  if (hi < 0) return 0;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if ((tops[mid + 1] ?? 0) <= y) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/** The entries to render for a scroll position, with some overscan. */
export function visibleRange(
  tops: Float64Array,
  scrollTop: number,
  viewport: number,
  overscan = 6,
): [number, number] {
  const count = tops.length - 1;
  if (count <= 0) return [0, 0];
  const first = entryAt(tops, scrollTop);
  const last = entryAt(tops, scrollTop + viewport);
  return [Math.max(0, first - overscan), Math.min(count, last + 1 + overscan)];
}

/** The scroll position that brings an entry fully into view (or the same). */
export function scrollToReveal(
  tops: Float64Array,
  index: number,
  scrollTop: number,
  viewport: number,
  padTop = 0,
): number {
  const top = tops[index] ?? 0;
  const bottom = tops[index + 1] ?? top;
  if (top - padTop < scrollTop) return Math.max(0, top - padTop);
  if (bottom > scrollTop + viewport) return bottom - viewport;
  return scrollTop;
}
