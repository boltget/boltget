// Queues in the list (APP-DESIGN, "Queues as built"): a queue view shows a
// queue's downloads in its order, and the user reorders them by dragging
// or with Alt+arrow keys. These are the pure rules for that; the engine
// keeps the order (docs/ENGINE.md, "Queues").
import type { DownloadId, DownloadState, QueueId } from "../bindings";

/** What the order needs to know of a download. */
export interface QueueItem {
  id: DownloadId;
  state: DownloadState;
  queue: QueueId;
  queuePosition: number;
}

/** Downloads that take part in a queue's order: all but completed ones. */
export function inQueue(item: Pick<QueueItem, "state" | "queue">, queue: QueueId): boolean {
  return item.queue === queue && item.state !== "completed";
}

/** A queue's downloads, in its order. */
export function queueOrder(items: readonly QueueItem[], queue: QueueId): DownloadId[] {
  return items
    .filter((i) => inQueue(i, queue))
    .sort((a, b) => a.queuePosition - b.queuePosition || a.id - b.id)
    .map((i) => i.id);
}

/**
 * The order after moving `ids` before `before` (`null`: to the end), as the
 * engine places them: in the order given, ids from elsewhere included.
 */
export function placeOrder(
  order: readonly DownloadId[],
  ids: readonly DownloadId[],
  before: DownloadId | null,
): DownloadId[] {
  const moving = new Set(ids);
  const rest = order.filter((id) => !moving.has(id));
  const at = before === null || moving.has(before) ? -1 : rest.indexOf(before);
  const unique = [...new Set(ids)];
  const index = at < 0 ? rest.length : at;
  return [...rest.slice(0, index), ...unique, ...rest.slice(index)];
}

export type Shift = "up" | "down" | "top" | "bottom";

/**
 * Moving the selected downloads one place (or to an end) as one block:
 * which ids move, in order, and before which download (`null`: to the
 * end). `null` when nothing would change.
 */
export function shiftSelection(
  order: readonly DownloadId[],
  selected: ReadonlySet<DownloadId>,
  by: Shift,
): { ids: DownloadId[]; before: DownloadId | null } | null {
  const ids = order.filter((id) => selected.has(id));
  if (ids.length === 0) return null;
  const first = order.indexOf(ids[0] as DownloadId);
  const last = order.indexOf(ids[ids.length - 1] as DownloadId);
  const rest = order.filter((id) => !selected.has(id));
  let before: DownloadId | null;
  switch (by) {
    case "top":
      before = rest[0] ?? null;
      break;
    case "bottom":
      before = null;
      break;
    case "up": {
      // Before the nearest download above the block.
      const above = order.slice(0, first).filter((id) => !selected.has(id));
      if (above.length === 0)
        return contiguous(order, ids) ? null : { ids, before: rest[0] ?? null };
      before = above[above.length - 1] as DownloadId;
      break;
    }
    case "down": {
      // After the nearest download below the block.
      const below = order.slice(last + 1).filter((id) => !selected.has(id));
      if (below.length === 0) return contiguous(order, ids) ? null : { ids, before: null };
      const i = rest.indexOf(below[0] as DownloadId);
      before = rest[i + 1] ?? null;
      break;
    }
  }
  const next = placeOrder(order, ids, before);
  return next.every((id, i) => id === order[i]) ? null : { ids, before };
}

function contiguous(order: readonly DownloadId[], ids: readonly DownloadId[]): boolean {
  const start = order.indexOf(ids[0] as DownloadId);
  return ids.every((id, i) => order[start + i] === id);
}

/**
 * Where dragged downloads land when dropped at slot `slot` of the order
 * (0 = before the first, `order.length` = after the last): before the first
 * download at or after the slot that is not being dragged.
 */
export function dropBefore(
  order: readonly DownloadId[],
  slot: number,
  dragged: ReadonlySet<DownloadId>,
): DownloadId | null {
  for (let i = Math.max(0, slot); i < order.length; i += 1) {
    const id = order[i] as DownloadId;
    if (!dragged.has(id)) return id;
  }
  return null;
}

/** How many downloads each queue holds (not counting completed ones). */
export function queueCounts(
  items: readonly Pick<QueueItem, "state" | "queue">[],
): Map<QueueId, number> {
  const counts = new Map<QueueId, number>();
  for (const i of items) {
    if (i.state !== "completed") counts.set(i.queue, (counts.get(i.queue) ?? 0) + 1);
  }
  return counts;
}

/** Minutes after midnight as the time input's value ("07:05"). */
export function timeValue(minutes: number | null): string {
  if (minutes === null) return "";
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;
}

/** A time input's value ("07:05") as minutes after midnight; `null` if empty or invalid. */
export function parseTime(value: string): number | null {
  const m = /^(\d{1,2}):(\d{2})$/.exec(value.trim());
  if (!m) return null;
  const h = Number(m[1]);
  const min = Number(m[2]);
  return h < 24 && min < 60 ? h * 60 + min : null;
}

/**
 * A `datetime-local` input's value for a time (local), and back. The page
 * sends Unix milliseconds; the input speaks local wall-clock time.
 */
export function dateTimeValue(ms: number): string {
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

export function parseDateTime(value: string): number | null {
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}/.test(value)) return null;
  const ms = new Date(value).getTime();
  return Number.isNaN(ms) ? null : ms;
}

/** A sensible first suggestion for "start later": the next full hour. */
export function nextHour(now: number): number {
  const d = new Date(now);
  d.setMinutes(0, 0, 0);
  d.setHours(d.getHours() + 1);
  return d.getTime();
}
