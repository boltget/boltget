// The Home hero's live app (components/home/HeroApp.astro): the numbers and
// shapes its window shows, shared by the build (the still frame every visitor
// gets first) and the browser script that plays a download. Ported from the
// desktop app (apps/desktop/src/lib/format.ts, details/fileMap.ts,
// details/Sparkline.svelte) so the recreation reads like the real thing; the
// downloads themselves come from the seeded simulation in sim.ts.
import type en from "../i18n/en.json";
import { DownloadSim, type SimOptions } from "./sim";

export type Ui = (typeof en)["home"]["app"]["ui"];

const MIB = 1024 * 1024;
const UNITS = ["b", "kib", "mib", "gib"] as const;

const fill = (message: string, params: Record<string, string | number>) =>
  message.replace(/\{(\w+)\}/g, (_, k: string) => String(params[k] ?? ""));

const numbers = new Map<string, Intl.NumberFormat>();
function num(value: number, digits: number, fixed: boolean): string {
  const key = `${digits}${fixed ? "f" : ""}`;
  let format = numbers.get(key);
  if (!format) {
    format = new Intl.NumberFormat("en", { maximumFractionDigits: digits, minimumFractionDigits: fixed ? digits : 0 });
    numbers.set(key, format);
  }
  return format.format(value);
}

function unitOf(bytes: number): number {
  let unit = 0;
  let value = Math.abs(bytes);
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return unit;
}

/** 2 decimals under 10, 1 under 100, then none; `fixed` keeps trailing zeros for live numbers. */
function scaled(bytes: number, unit: number, fixed = false): string {
  const value = bytes / 1024 ** unit;
  if (unit === 0) return num(value, 0, false);
  const abs = Math.abs(value);
  return num(value, abs < 10 ? 2 : abs < 100 ? 1 : 0, fixed);
}

/** A big number and the small text after it ("9.02" + " MiB/s"). */
export interface Parts {
  value: string;
  after: string;
}

/** The app's units and phrases, from the site's catalog. Sizes are in MiB. */
export class Fmt {
  constructor(readonly ui: Ui) {}

  bytes(mib: number): string {
    const b = mib * MIB;
    const u = unitOf(b);
    return fill(this.ui.units[UNITS[u] ?? "b"], { value: scaled(b, u) });
  }
  speed(mibps: number): string {
    const b = mibps * MIB;
    const u = unitOf(b);
    return fill(this.ui.speeds[UNITS[u] ?? "b"], { value: scaled(b, u, true) });
  }
  speedParts(mibps: number): Parts {
    const b = mibps * MIB;
    const u = unitOf(b);
    return { value: scaled(b, u, true), after: fill(this.ui.speeds[UNITS[u] ?? "b"], { value: "" }) };
  }
  progress(done: number, total: number): string {
    const u = unitOf(total * MIB);
    return fill(this.ui.progress, { done: scaled(Math.min(done, total) * MIB, u, true), total: this.bytes(total) });
  }
  progressParts(done: number, total: number): Parts {
    const u = unitOf(total * MIB);
    return { value: scaled(Math.min(done, total) * MIB, u, true), after: fill(this.ui.progress, { done: "", total: this.bytes(total) }) };
  }
  bytesParts(mib: number): Parts {
    const b = mib * MIB;
    const u = unitOf(b);
    return { value: scaled(b, u), after: fill(this.ui.units[UNITS[u] ?? "b"], { value: "" }) };
  }
  eta(seconds: number): string {
    const s = Math.max(1, Math.round(seconds));
    return s < 60 ? fill(this.ui.eta.seconds, { count: s }) : fill(this.ui.eta.minutes, { count: Math.round(s / 60) });
  }
  duration(seconds: number): string {
    const s = Math.max(1, Math.round(seconds));
    return s < 60 ? fill(this.ui.duration.seconds, { count: s }) : fill(this.ui.duration.minutes, { count: Math.round(s / 60) });
  }
  join(first: string, second: string): string {
    return fill(this.ui.join, { first, second });
  }
  percent(value: number): string {
    return fill(this.ui.percent, { value: Math.floor(value) });
  }
  mapNote(count: number): string {
    return fill(count === 1 ? this.ui.mapNote.one : this.ui.mapNote.other, { count });
  }
  peak(mibps: number): string {
    return fill(this.ui.peak, { speed: this.speed(mibps) });
  }
}

/** The details pane's file map grid (fileMap.ts): 8 rows of ~9 px squares in the pane's 320 px. */
export interface Grid {
  cols: number;
  rows: number;
  cell: number;
  gap: number;
  left: number;
  width: number;
  height: number;
}

export function grid(width = 320, rows = 8, target = 9, gap = 2): Grid {
  const cols = Math.max(1, Math.floor((width + gap) / (target + gap)));
  const cell = Math.max(2, Math.floor((width - gap * (cols - 1)) / cols));
  const used = cols * cell + (cols - 1) * gap;
  return { cols, rows, cell, gap, left: Math.floor((width - used) / 2), width, height: rows * cell + (rows - 1) * gap };
}

/** The app's map colours (light theme): along the graphite scale toward ink (graphite-400 to -800), the head in ink. */
export const MAP_RAMP: readonly (readonly [number, number, number])[] = [
  [160, 163, 168],
  [125, 128, 134],
  [70, 72, 76],
  [46, 48, 51],
];

export function ramp(u: number): [number, number, number] {
  const x = Math.max(0, Math.min(1, u)) * (MAP_RAMP.length - 1);
  const i = Math.min(MAP_RAMP.length - 2, Math.floor(x));
  const k = x - i;
  const a = MAP_RAMP[i]!;
  const b = MAP_RAMP[i + 1]!;
  return [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k];
}

/** Where a cell sits along the field's gradient: mostly left to right. */
export function rampPosition(col: number, row: number, cols: number, rows: number): number {
  return (col / Math.max(1, cols - 1)) * 0.78 + (row / Math.max(1, rows - 1)) * 0.22;
}

export function hash(i: number): number {
  let h = Math.imul(i ^ 0x9e3779b9, 0x85ebca6b);
  h ^= h >>> 13;
  h = Math.imul(h, 0xc2b2ae35);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
}

/** Per cell: how much has arrived (0..1) and whether a connection is writing there. */
export function cells(sim: DownloadSim, count: number): { fill: Float32Array; head: Uint8Array } {
  const fillArr = new Float32Array(count);
  const head = new Uint8Array(count);
  const slice = sim.o.total / count;
  for (let i = 0; i < count; i++) {
    const c = sim.cell(i * slice, (i + 1) * slice);
    fillArr[i] = sim.finished ? 1 : c.f;
    head[i] = c.head && !sim.finished ? 1 : 0;
  }
  return { fill: fillArr, head };
}

/** Connections at work: their number as shown (from 1) and their part's progress. */
export function workers(sim: DownloadSim): { id: number; frac: number }[] {
  return sim.segs
    .filter((s) => !s.done)
    .map((s) => ({ id: s.conn + 1, frac: (s.pos - s.start) / Math.max(1e-9, s.end - s.start) }))
    .sort((a, b) => a.id - b.id);
}

export const SPARK_W = 320;
export const SPARK_H = 56;

/** The sparkline's line, area and newest point (Sparkline.svelte), or null. */
export function spark(samples: readonly number[], capacity = 120, minimum = 20) {
  if (samples.length < 2) return null;
  const max = Math.max(...samples) * 1.15 || 1;
  const span = Math.min(capacity, Math.max(minimum, samples.length));
  const step = SPARK_W / Math.max(1, span - 1);
  const offset = (span - samples.length) * step;
  const points = samples.map((v, i) => [offset + i * step, SPARK_H - 2 - (v / max) * (SPARK_H - 6)] as const);
  const line = points.map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)} ${y.toFixed(1)}`).join("");
  const first = points[0]!;
  const last = points[points.length - 1]!;
  return { line, area: `${line}L${last[0].toFixed(1)} ${SPARK_H}L${first[0].toFixed(1)} ${SPARK_H}Z`, head: last };
}

/** One file of the demo: its size in MiB and the line it arrives over. */
export interface DemoFile {
  total: number;
  /** Shared link cap and per-connection rates, MiB/s. */
  cap: number;
  rate: [number, number];
  seed: number;
}

/** The three files the visitor can pick (order matches home.app.chips). */
export const CHIP_FILES: Record<"iso" | "film" | "game", DemoFile> = {
  iso: { total: 1198, cap: 96, rate: [14, 20], seed: 5 },
  film: { total: 3953, cap: 112, rate: [16, 24], seed: 9 },
  game: { total: 1413, cap: 88, rate: [13, 19], seed: 13 },
};

/** Wall-clock seconds a picked download takes; the simulation is sped up to fit. */
export const PLAY_SECONDS = 8.5;

/** How much faster than real time a file plays. */
export function paceOf(file: DemoFile): number {
  return Math.max(1, file.total / (file.cap * 0.92) / PLAY_SECONDS);
}

export function simFor(file: DemoFile, pace = 1): DownloadSim {
  const o: SimOptions = {
    total: file.total,
    start: 4,
    max: 16,
    rate: file.rate,
    cap: file.cap,
    every: 0.6 * pace,
    minSplit: file.total * 0.01,
    seed: file.seed,
  };
  return new DownloadSim(o);
}

/** The selected download in the still frame: big_buck_bunny, a little past half. */
export const BUNNY: DemoFile = { total: 691, cap: 9.4, rate: [1.4, 2.1], seed: 11 };
export const BUNNY_AT = 389;

/** Samples every half second of simulated time, from the sim's 0.1 s history. */
export function samplesOf(sim: DownloadSim): number[] {
  return sim.history.filter((_, i) => i % 5 === 4);
}
