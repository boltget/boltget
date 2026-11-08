// Markup for the Home hero's live app (components/home/HeroApp.astro). The
// build and the browser script render the window's rows and details pane
// through these same functions, so the still frame and the live one cannot
// drift apart. Every text passed in comes from the catalog (home.app.*).
import type { DownloadSim } from "./sim";
import { cells, hash, ramp, rampPosition, spark, SPARK_H, SPARK_W, type Fmt, type Grid, type Parts, type Ui } from "./appdemo";

/** The shell's 16 px line icons (apps/desktop/src/lib/Icon.svelte). */
const ICONS = {
  all: "M2.5 4.5h11M2.5 8h11M2.5 11.5h11",
  active: "M8 2.5v7.5M4.75 6.75 8 10l3.25-3.25M2.5 12v1.5h11V12",
  completed: "M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11ZM5.5 8.25l1.75 1.75 3.25-3.5",
  scheduled: "M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11ZM8 5v3.25l2.25 1.5",
  video: "M2.5 3.5h11v9h-11ZM6.75 6v4l3.25-2Z",
  music:
    "M6 11.5V3.5l7.5-1.5v8M6 11.5a1.75 1.75 0 1 1-3.5 0 1.75 1.75 0 0 1 3.5 0ZM13.5 10a1.75 1.75 0 1 1-3.5 0 1.75 1.75 0 0 1 3.5 0Z",
  archives: "M2.5 3h11v3h-11ZM3.5 6v7h9V6M6.5 8.5h3",
  programs: "M2.5 3.5h11v9h-11ZM2.5 6h11M4.5 4.75h1",
  documents: "M3.5 2h5.5l3.5 3.5v8.5h-9ZM9 2v3.5h3.5M5.5 8.5h5M5.5 11h3.5",
  other: "M2 4h4.5l1.5 1.5h6v7.5H2Z",
  settings:
    "M2.5 5h6M11.5 5h2M2.5 11h2M7.5 11h6M8.5 5a1.5 1.5 0 1 0 3 0 1.5 1.5 0 0 0-3 0ZM4.5 11a1.5 1.5 0 1 0 3 0 1.5 1.5 0 0 0-3 0Z",
  search: "M7 2.5a4.5 4.5 0 1 0 0 9 4.5 4.5 0 0 0 0-9ZM10.25 10.25 13.5 13.5",
  plus: "M8 3v10M3 8h10",
  pause: "M5.5 3.5v9M10.5 3.5v9",
  folder: "M2 4h4.5l1.5 1.5h6v7.5H2Z",
  open: "M9 2.5h4.5V7M13.5 2.5 7.5 8.5M11.5 9.5v4h-9v-9h4",
  close: "M4 4l8 8M12 4l-8 8",
  more: "M3 7.5h1v1H3ZM7.5 7.5h1v1h-1ZM12 7.5h1v1h-1Z",
  check: "M3.5 8.5l3 3 6-6.5",
  sort: "M5 3v10M2.5 10.5 5 13l2.5-2.5M11 13V3M8.5 5.5 11 3l2.5 2.5",
  pane: "M2.5 3.5h11v9h-11ZM9.5 3.5v9",
  queue: "M2.5 4h7M2.5 8h7M2.5 12h7M12.5 3v10M11 11.5l1.5 1.5 1.5-1.5",
  copy: "M5.5 5.5h8v8h-8ZM10.5 5.5v-3h-8v8h3",
  down: "M3.5 6 8 10.5 12.5 6",
} as const;

export type AppIcon = keyof typeof ICONS;

export function icon(name: AppIcon, size = 16): string {
  return `<svg class="i" viewBox="0 0 16 16" width="${size}" height="${size}" aria-hidden="true"><path d="${ICONS[name]}"/></svg>`;
}

/** The category icon for a file name (after apps/desktop/src/lib/list/category.ts). */
export function categoryIcon(name: string): AppIcon {
  const ext = name.slice(name.lastIndexOf(".") + 1).toLowerCase();
  if (["mov", "mkv", "mp4", "webm"].includes(ext)) return "video";
  if (["flac", "mp3", "ogg"].includes(ext)) return "music";
  if (["exe", "msi"].includes(ext)) return "programs";
  if (["pdf", "docx", "txt"].includes(ext)) return "documents";
  if (["zip", "iso", "7z", "tar", "gz"].includes(ext)) return "archives";
  return "other";
}

const ESC: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" };
export const esc = (s: string) => s.replace(/[&<>"]/g, (c) => ESC[c] ?? c);

export interface RowModel {
  id: string;
  name: string;
  state: "downloading" | "paused" | "completed";
  /** 0..1 for the bar. */
  fraction: number;
  status: string;
  /** The right edge at rest: a percentage, "Paused", "Done" or a time. */
  rest: string;
  restIcon?: "pause" | "check";
}

/** One list row (apps/desktop/src/lib/list/Row.svelte, comfortable density). */
export function rowHtml(r: RowModel, selected = false): string {
  const bar =
    r.state === "completed"
      ? ""
      : `<span class="bar${r.state === "downloading" ? " live" : ""}"><span class="fill" style="transform:scaleX(${r.fraction.toFixed(4)})"></span></span>`;
  const sign = r.restIcon ? `<span class="sign ${r.restIcon}">${icon(r.restIcon, 14)}</span>` : "";
  const action = r.state === "completed" ? "open" : "pause";
  return `<div class="row${selected ? " selected" : ""}" data-row="${esc(r.id)}" data-state="${r.state}"><span class="type">${icon(categoryIcon(r.name))}</span><div class="text"><span class="name">${esc(r.name)}</span><span class="line2">${bar}<span class="status">${esc(r.status)}</span></span></div><span class="end"><span class="rest">${sign}${esc(r.rest)}</span><span class="acts"><span class="ib">${icon(action)}</span><span class="ib">${icon("more")}</span></span></span></div>`;
}

export interface Stat {
  label: string;
  parts: Parts;
}

export function statsHtml(stats: readonly Stat[]): string {
  return stats
    .map((s) => `<div class="stat"><dt>${esc(s.label)}</dt><dd><span class="v"><b>${esc(s.parts.value)}</b>${esc(s.parts.after)}</span></dd></div>`)
    .join("");
}

export function workersHtml(list: readonly { id: number; frac: number }[], fmt: Fmt): string {
  return list
    .map(
      (w) =>
        `<li><span class="wid">#${w.id}</span><span class="wbar"><span style="transform:scaleX(${w.frac.toFixed(4)})"></span></span><span class="wleft">${esc(fmt.percent(w.frac * 100))}</span></li>`,
    )
    .join("");
}

/** The sparkline's SVG and its head dot (empty paths before two samples). */
export function sparkHtml(samples: readonly number[]): string {
  const p = spark(samples);
  const head = p
    ? `<span class="shead" style="left:${((p.head[0] / SPARK_W) * 100).toFixed(2)}%;top:${((p.head[1] / SPARK_H) * 100).toFixed(2)}%"></span>`
    : "";
  return `<svg class="spark" viewBox="0 0 ${SPARK_W} ${SPARK_H}" preserveAspectRatio="none" aria-hidden="true"><path class="grid" d="M0 ${SPARK_H / 2}H${SPARK_W}"/><path class="area" d="${p?.area ?? ""}"/><path class="line" d="${p?.line ?? ""}"/></svg>${head}`;
}

/** A still of the file map as SVG squares (the canvas draws the live one). */
export function mapSvg(sim: DownloadSim, g: Grid, settled = false): string {
  const n = g.cols * g.rows;
  const { fill: f, head } = cells(sim, n);
  let out = "";
  for (let i = 0; i < n; i++) {
    const col = i % g.cols;
    const row = Math.floor(i / g.cols);
    const x = g.left + col * (g.cell + g.gap);
    const y = row * (g.cell + g.gap);
    const v = hash(i);
    out += `<rect x="${x}" y="${y}" width="${g.cell}" height="${g.cell}" fill="#064E3B" fill-opacity="${(0.07 * (0.7 + 0.6 * v)).toFixed(3)}"/>`;
    const fv = f[i] ?? 0;
    if (fv <= 0) continue;
    const [r, gg, b] = ramp(rampPosition(col, row, g.cols, g.rows));
    const alpha = settled ? 0.42 + 0.1 * v : fv >= 1 ? 1 : 0.16 + 0.34 * fv;
    out += `<rect x="${x}" y="${y}" width="${g.cell}" height="${g.cell}" fill="rgb(${r | 0},${gg | 0},${b | 0})" fill-opacity="${alpha.toFixed(3)}"/>`;
    if (head[i]) {
      out += `<rect x="${x - 1}" y="${y - 1}" width="${g.cell + 2}" height="${g.cell + 2}" fill="#10B981" fill-opacity="0.24"/><rect x="${x}" y="${y}" width="${g.cell}" height="${g.cell}" fill="#022C22"/>`;
    }
  }
  return `<svg class="still" viewBox="0 0 ${g.width} ${g.height}" width="${g.width}" height="${g.height}" aria-hidden="true">${out}</svg>`;
}

export interface PaneModel {
  name: string;
  sub: string;
  complete: boolean;
  stats: Stat[];
  note: string;
  samples: number[];
  peak: string;
  workers: { id: number; frac: number }[];
  pathLabel: string;
  path: string;
  url: string;
  /** The file map's markup (an SVG still or a canvas). */
  map: string;
}

/** A path that may break after each separator, like the app's breakable(). */
const breakable = (s: string) => esc(s).replace(/([/\\])/g, "$1<wbr>");

/** The details pane's content (apps/desktop/src/lib/details/DetailsPane.svelte). */
export function paneHtml(p: PaneModel, ui: Ui, fmt: Fmt): string {
  const action = p.complete
    ? `<span class="btn primary">${icon("open")}${esc(ui.open)}</span>`
    : `<span class="btn secondary">${icon("pause")}${esc(ui.pause)}</span>`;
  const note = p.complete
    ? `<span class="note ok" data-p="note">${icon("check", 14)}${esc(p.note)}</span>`
    : `<span class="note" data-p="note">${esc(p.note)}</span>`;
  const live = p.complete
    ? ""
    : `<section class="block"><div class="lbl"><h3>${esc(ui.throughput)}</h3><span class="note" data-p="peak">${esc(p.peak)}</span></div><div class="sparkwrap" data-p="spark">${sparkHtml(p.samples)}</div></section>` +
      `<section class="block"><div class="lbl"><h3>${esc(ui.connections)}</h3></div><ul class="workers" data-p="workers">${workersHtml(p.workers, fmt)}</ul></section>`;
  return (
    `<header class="phead"><div class="title-row"><span class="type">${icon(categoryIcon(p.name), 20)}</span><div class="titles"><h2 class="pname">${esc(p.name)}</h2><p class="sub">${esc(p.sub)}</p></div><span class="ib">${icon("close")}</span></div>` +
    `<div class="pactions">${action}<span class="ib">${icon("folder")}</span><span class="ib">${icon("more")}</span></div></header>` +
    `<dl class="stats" data-p="stats">${statsHtml(p.stats)}</dl>` +
    `<section class="block"><div class="lbl"><h3>${esc(ui.map)}</h3>${note}</div><div class="map" data-p="map">${p.map}</div></section>${live}` +
    `<section class="block"><div class="lbl"><h3>${esc(ui.file)}</h3></div><dl class="facts">` +
    `<div class="fact"><dt>${esc(p.pathLabel)}</dt><dd class="mono">${breakable(p.path)}</dd><span class="ib">${icon("copy")}</span></div>` +
    `<div class="fact"><dt>${esc(ui.link)}</dt><dd class="mono">${breakable(p.url)}</dd><span class="ib">${icon("copy")}</span></div></dl></section>`
  );
}

/** The running pane's stats for `sim`. */
export function runningStats(sim: DownloadSim, ui: Ui, fmt: Fmt): Stat[] {
  const eta = sim.eta();
  return [
    { label: ui.speed, parts: fmt.speedParts(sim.speed) },
    { label: ui.received, parts: fmt.progressParts(sim.downloaded, sim.o.total) },
    { label: ui.left, parts: { value: eta === null ? "–" : fmt.duration(eta), after: "" } },
    { label: ui.connections, parts: { value: String(sim.active), after: "" } },
  ];
}

/** The finished pane's stats. */
export function doneStats(total: number, finished: string, ui: Ui, fmt: Fmt): Stat[] {
  return [
    { label: ui.size, parts: fmt.bytesParts(total) },
    { label: ui.finished, parts: { value: finished, after: "" } },
  ];
}

/** The list row's status line while it downloads. */
export function runningStatus(sim: DownloadSim, fmt: Fmt): string {
  let text = fmt.progress(sim.downloaded, sim.o.total);
  if (sim.speed > 0.05) text = fmt.join(text, fmt.speed(sim.speed));
  const eta = sim.eta();
  if (eta !== null) text = fmt.join(text, fmt.eta(eta));
  return text;
}
