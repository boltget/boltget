// Draws a file map: a grid of square chunks, each a slice of the file, filled
// as a simulated download (sim.ts) writes it. Shared by the hero demo and the
// "How it works" scene on Home.
//
// The look follows brand/BRAND.md, "Pixels on the web": one continuous
// gradient flows across the whole field (each square is a window onto it, not
// a flat swatch); a square eases in when its bytes land and carries a brief
// light that fades; arrived squares have a faint inner light at the top and a
// hairline of depth at the bottom; empty squares vary a little in tone so the
// field reads as material, not a spreadsheet. Squares stay square, with gaps.
import type { DownloadSim } from "./sim";

export interface MapLayout {
  cols: number;
  rows: number;
  /** Gap between squares in CSS pixels. */
  gap: number;
  /**
   * "row": the file runs left to right, then down (the app's file map).
   * "column": it runs top to bottom, then right, so every segment is a
   * horizontal span (the "How it works" bar).
   */
  order: "row" | "column";
  /** Extra space above the squares, for segment brackets (CSS pixels). */
  top?: number;
}

/** The field's gradient, mint to deep emerald through teal (data-series colours). */
const RAMP: readonly (readonly [number, number, number])[] = [
  [52, 211, 153], // emerald-400
  [16, 185, 129], // emerald-500
  [5, 150, 105], // emerald-600
  [13, 148, 136], // atmosphere teal
  [4, 120, 87], // emerald-700
];

function ramp(u: number): [number, number, number] {
  const x = Math.max(0, Math.min(1, u)) * (RAMP.length - 1);
  const i = Math.min(RAMP.length - 2, Math.floor(x));
  const k = x - i;
  const a = RAMP[i]!;
  const b = RAMP[i + 1]!;
  return [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k];
}

function hash(i: number): number {
  let h = Math.imul(i ^ 0x9e3779b9, 0x85ebca6b);
  h ^= h >>> 13;
  h = Math.imul(h, 0xc2b2ae35);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
}

const easeOut = (x: number) => 1 - Math.pow(1 - Math.max(0, Math.min(1, x)), 3);

export class ChunkMap {
  readonly canvas: HTMLCanvasElement;
  layout: MapLayout;
  /** Wall-clock time (s) at which each square was first complete, or -1. */
  private arrived = new Float64Array(0);
  private lastT = -1;
  private lastDraw = 0;
  cell = 10;
  /** True while a square is still easing in (keep drawing to finish it). */
  busy = false;

  constructor(canvas: HTMLCanvasElement, layout: MapLayout) {
    this.canvas = canvas;
    this.layout = layout;
  }

  /** Forget arrival times (a new run, or a rewind). */
  reset(): void {
    this.arrived.fill(-1);
    this.lastT = -1;
  }

  /** Index of the square at grid position (col, row) in file order. */
  private index(col: number, row: number): number {
    const { cols, rows, order } = this.layout;
    return order === "row" ? row * cols + col : col * rows + row;
  }

  /** Sizes the canvas to its CSS width; returns the CSS height it needs. */
  measure(): number {
    const { cols, rows, gap } = this.layout;
    const top = this.layout.top ?? 0;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const width = this.canvas.clientWidth;
    this.cell = (width - gap * (cols - 1)) / cols;
    const height = Math.round(top + rows * this.cell + (rows - 1) * gap);
    const w = Math.round(width * dpr);
    const h = Math.round(height * dpr);
    if (this.canvas.width !== w || this.canvas.height !== h) {
      this.canvas.width = w;
      this.canvas.height = h;
      this.canvas.style.height = `${height}px`;
    }
    if (this.arrived.length !== cols * rows) this.arrived = new Float64Array(cols * rows).fill(-1);
    return height;
  }

  /** x of the left edge of column `col`, CSS pixels. */
  colX(col: number): number {
    return col * (this.cell + this.layout.gap);
  }

  /** Column holding file offset `mib` (column order only). */
  colOf(mib: number, total: number): number {
    const n = this.layout.cols * this.layout.rows;
    return Math.floor((Math.max(0, Math.min(total - 1e-9, mib)) / total) * n / this.layout.rows);
  }

  /** Draws the map for `sim` at its current time. */
  draw(sim: DownloadSim, opts: { heads?: boolean } = {}): CanvasRenderingContext2D | null {
    const ctx = this.canvas.getContext("2d");
    if (!ctx) return null;
    if (this.arrived.length === 0) this.measure();
    const { cols, rows, gap } = this.layout;
    const top = this.layout.top ?? 0;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const width = this.canvas.width / dpr;
    const height = this.canvas.height / dpr;
    const cell = this.cell;
    if (sim.t < this.lastT) this.reset();
    // On a first draw (or after a rewind), squares already there are settled.
    // Easing runs on the wall clock, so it completes even when the caller
    // holds the simulation still (the scroll-driven scene).
    // A draw that is not part of a running animation (a still, a resize) also
    // settles whatever arrived since the last one.
    const now = performance.now() / 1000;
    const fresh = this.lastT < 0 || now - this.lastDraw > 0.25;
    this.lastT = sim.t;
    this.lastDraw = now;
    this.busy = false;

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, width, height);
    const n = cols * rows;
    const slice = sim.o.total / n;
    const heads = opts.heads ?? true;
    const band = Math.max(1, cell * 0.38);

    for (let row = 0; row < rows; row++) {
      for (let col = 0; col < cols; col++) {
        const i = this.index(col, row);
        const x = col * (cell + gap);
        const y = top + row * (cell + gap);
        const c = sim.cell(i * slice, (i + 1) * slice);
        const h = hash(i);

        // Empty: a pale, slightly uneven ground.
        ctx.globalAlpha = 0.62 + 0.38 * h;
        ctx.fillStyle = "#E9EFEC";
        ctx.fillRect(x, y, cell, cell);

        if (c.f <= 0) continue;
        const u = (col / Math.max(1, cols - 1)) * 0.78 + (row / Math.max(1, rows - 1)) * 0.22;
        let [r, g, b] = ramp(u);
        let alpha: number;
        if (c.f >= 1) {
          if (this.arrived[i]! < 0) this.arrived[i] = fresh ? now - 10 : now;
          const age = now - this.arrived[i]!;
          if (age < 0.9) this.busy = true;
          alpha = 0.35 + 0.65 * easeOut(age / 0.45);
          // A brief light as the square lands, fading over ~0.5 s.
          const flash = Math.exp(-age / 0.32) * 0.55;
          r += (236 - r) * flash;
          g += (253 - g) * flash;
          b += (245 - b) * flash;
        } else {
          alpha = 0.16 + 0.34 * c.f;
        }
        ctx.globalAlpha = alpha;
        ctx.fillStyle = `rgb(${r | 0},${g | 0},${b | 0})`;
        ctx.fillRect(x, y, cell, cell);
        if (c.f >= 1) {
          // Inner light at the top, a hairline of depth at the bottom.
          ctx.globalAlpha = alpha * 0.13;
          ctx.fillStyle = "#FFFFFF";
          ctx.fillRect(x, y, cell, band);
          ctx.globalAlpha = alpha * 0.14;
          ctx.fillStyle = "#022C22";
          ctx.fillRect(x, y + cell - Math.max(1, cell * 0.08), cell, Math.max(1, cell * 0.08));
        }
        if (heads && c.head && !sim.finished) {
          // The square a connection is writing: deep ink with a soft halo.
          ctx.globalAlpha = 0.22;
          ctx.fillStyle = "#10B981";
          ctx.fillRect(x - gap * 0.5, y - gap * 0.5, cell + gap, cell + gap);
          ctx.globalAlpha = 1;
          ctx.fillStyle = "#06261E";
          ctx.fillRect(x, y, cell, cell);
        }
      }
    }
    ctx.globalAlpha = 1;
    return ctx;
  }
}
