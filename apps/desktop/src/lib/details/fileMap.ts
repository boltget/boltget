// The file map's numbers (APP-DESIGN #11): the file cut into a grid of
// square cells, how much of each cell has arrived, and which connection is
// writing where. Pure, so the drawing code (FileMap.svelte) stays small and
// this part is tested.
import type { SegmentSpan } from "../bindings";

export interface Grid {
  cols: number;
  rows: number;
  /** Square size and gap, whole CSS pixels. */
  cell: number;
  gap: number;
  /** Left offset that centres the grid in the width. */
  left: number;
  width: number;
  height: number;
}

/** A grid of `rows` rows that fills `width` with squares near `target` px. */
export function grid(width: number, rows: number, target = 9, gap = 2): Grid {
  const usable = Math.max(0, Math.floor(width));
  const cols = Math.max(1, Math.floor((usable + gap) / (target + gap)));
  const cell = Math.max(2, Math.floor((usable - gap * (cols - 1)) / cols));
  const used = cols * cell + (cols - 1) * gap;
  return {
    cols,
    rows,
    cell,
    gap,
    left: Math.floor((usable - used) / 2),
    width: usable,
    height: rows * cell + (rows - 1) * gap,
  };
}

export interface Coverage {
  /** 0 to 1 per cell: how much of its bytes arrived. */
  fill: Float32Array;
  /**
   * The connection writing in each cell, numbered from 1 as shown to people
   * (the engine counts from 0); 0 is none.
   */
  heads: Int32Array;
  /** The connection that owns each cell's range, numbered the same way. */
  owners: Int32Array;
}

/** How much of each of `cells` equal slices of `total` bytes has arrived. */
export function coverage(segments: readonly SegmentSpan[], total: number, cells: number): Coverage {
  const fill = new Float32Array(cells);
  const heads = new Int32Array(cells);
  const owners = new Int32Array(cells);
  if (total <= 0 || cells <= 0) return { fill, heads, owners };
  const slice = total / cells;
  const cellOf = (byte: number) => Math.min(cells - 1, Math.max(0, Math.floor(byte / slice)));

  for (const s of segments) {
    const end = s.end ?? total;
    const done = Math.min(end, s.start + s.received);
    if (done > s.start) {
      for (let c = cellOf(s.start); c <= cellOf(done - 1); c += 1) {
        const a = Math.max(s.start, c * slice);
        const b = Math.min(done, (c + 1) * slice);
        if (b > a) fill[c] = (fill[c] ?? 0) + (b - a) / slice;
      }
    }
    if (s.connection !== null && end > s.start) {
      const shown = s.connection + 1;
      for (let c = cellOf(s.start); c <= cellOf(end - 1); c += 1) owners[c] = shown;
      if (done < end) heads[cellOf(done)] = shown;
    }
  }
  for (let c = 0; c < cells; c += 1) fill[c] = Math.min(1, fill[c] ?? 0);
  return { fill, heads, owners };
}

/** Without a segment table: the received share, filled in file order. */
export function contiguous(fraction: number, cells: number): Coverage {
  const fill = new Float32Array(cells);
  const exact = Math.max(0, Math.min(1, fraction)) * cells;
  for (let c = 0; c < cells; c += 1) fill[c] = Math.max(0, Math.min(1, exact - c));
  return { fill, heads: new Int32Array(cells), owners: new Int32Array(cells) };
}

/** The byte range `[from, to)` of a cell. */
export function cellRange(index: number, cells: number, total: number): [number, number] {
  const slice = total / cells;
  return [Math.floor(index * slice), Math.min(total, Math.floor((index + 1) * slice))];
}

/** The cell under a point (CSS pixels within the canvas), or -1. */
export function cellAt(g: Grid, x: number, y: number): number {
  const pitch = g.cell + g.gap;
  const col = Math.floor((x - g.left) / pitch);
  const row = Math.floor(y / pitch);
  if (col < 0 || col >= g.cols || row < 0 || row >= g.rows) return -1;
  return row * g.cols + col;
}

/** Where a cell sits along the field's gradient: mostly left to right. */
export function rampPosition(col: number, row: number, cols: number, rows: number): number {
  return (col / Math.max(1, cols - 1)) * 0.78 + (row / Math.max(1, rows - 1)) * 0.22;
}

/**
 * Connections at work, from the segment table: their number as shown (from
 * 1) and their part's progress.
 */
export function connections(
  segments: readonly SegmentSpan[] | null,
  total: number | null,
): { id: number; done: number; size: number }[] {
  if (!segments) return [];
  return segments
    .filter((s) => s.connection !== null)
    .map((s) => {
      const size = (s.end ?? total ?? s.start + s.received) - s.start;
      return { id: (s.connection as number) + 1, done: s.received, size };
    })
    .sort((a, b) => a.id - b.id);
}
