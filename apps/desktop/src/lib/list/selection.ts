// Selection in the main list, as on Windows: click selects one, Ctrl+click
// toggles, Shift+click and Shift+arrows extend from the anchor, Ctrl+A takes
// everything. `active` is the row the keyboard is on (the focused option).
// Pure: every function returns a new selection.
import type { DownloadId } from "../bindings";

export interface Selection {
  readonly ids: ReadonlySet<DownloadId>;
  /** Where a Shift range starts. */
  readonly anchor: DownloadId | null;
  /** The keyboard's row. */
  readonly active: DownloadId | null;
}

export const EMPTY: Selection = { ids: new Set(), anchor: null, active: null };

export interface Modifiers {
  shift?: boolean;
  ctrl?: boolean;
}

function only(id: DownloadId): Selection {
  return { ids: new Set([id]), anchor: id, active: id };
}

function range(order: readonly DownloadId[], from: DownloadId, to: DownloadId): DownloadId[] {
  const a = order.indexOf(from);
  const b = order.indexOf(to);
  if (a < 0 || b < 0) return [to];
  const [lo, hi] = a < b ? [a, b] : [b, a];
  return order.slice(lo, hi + 1);
}

/** A click (or Space with modifiers) on a row. */
export function click(
  selection: Selection,
  order: readonly DownloadId[],
  id: DownloadId,
  { shift = false, ctrl = false }: Modifiers = {},
): Selection {
  if (shift && selection.anchor !== null && order.includes(selection.anchor)) {
    const ids = new Set(ctrl ? selection.ids : []);
    for (const r of range(order, selection.anchor, id)) ids.add(r);
    return { ids, anchor: selection.anchor, active: id };
  }
  if (ctrl) {
    const ids = new Set(selection.ids);
    if (ids.has(id)) ids.delete(id);
    else ids.add(id);
    return { ids, anchor: id, active: id };
  }
  return only(id);
}

/** A right click: keeps a selection that contains the row, else selects it. */
export function contextClick(selection: Selection, id: DownloadId): Selection {
  if (selection.ids.has(id)) return { ...selection, active: id };
  return only(id);
}

export type Move = number | "home" | "end";

/** Arrow keys, Home, End, PageUp and PageDown (a page is `move` rows). */
export function move(
  selection: Selection,
  order: readonly DownloadId[],
  by: Move,
  { shift = false, ctrl = false }: Modifiers = {},
): Selection {
  if (order.length === 0) return EMPTY;
  const current = selection.active === null ? -1 : order.indexOf(selection.active);
  let index: number;
  if (by === "home") index = 0;
  else if (by === "end") index = order.length - 1;
  else if (current < 0) index = by > 0 ? 0 : order.length - 1;
  else index = Math.max(0, Math.min(order.length - 1, current + by));
  const id = order[index] as DownloadId;
  // Ctrl+arrows move the keyboard's row without changing the selection.
  if (ctrl && !shift) return { ...selection, active: id };
  if (shift) {
    const anchor =
      selection.anchor !== null && order.includes(selection.anchor) ? selection.anchor : id;
    return { ids: new Set(range(order, anchor, id)), anchor, active: id };
  }
  return only(id);
}

/** Ctrl+A. */
export function selectAll(selection: Selection, order: readonly DownloadId[]): Selection {
  if (order.length === 0) return EMPTY;
  return {
    ids: new Set(order),
    anchor: selection.anchor ?? order[0] ?? null,
    active: selection.active ?? order[0] ?? null,
  };
}

/**
 * Drops rows that are no longer shown (removed, or filtered out). With
 * `follow` (the row was removed), the keyboard's row and the selection move
 * to the row that took its place, so Delete can be pressed again.
 */
export function prune(
  selection: Selection,
  order: readonly DownloadId[],
  previousOrder: readonly DownloadId[] = order,
  follow = true,
): Selection {
  const present = new Set(order);
  const ids = new Set([...selection.ids].filter((id) => present.has(id)));
  let active = selection.active;
  if (active !== null && !present.has(active)) {
    const was = previousOrder.indexOf(active);
    const after = previousOrder.slice(was + 1).find((id) => present.has(id));
    const before = previousOrder
      .slice(0, Math.max(0, was))
      .reverse()
      .find((id) => present.has(id));
    active = was < 0 || !follow ? null : (after ?? before ?? null);
    if (active !== null && selection.ids.size > 0 && ids.size === 0) ids.add(active);
  }
  const anchor =
    selection.anchor !== null && present.has(selection.anchor) ? selection.anchor : active;
  if (
    ids.size === selection.ids.size &&
    active === selection.active &&
    anchor === selection.anchor
  ) {
    return selection;
  }
  return { ids, anchor, active };
}

/** The selected ids in display order. */
export function selectedInOrder(selection: Selection, order: readonly DownloadId[]): DownloadId[] {
  return order.filter((id) => selection.ids.has(id));
}
