// The Home hero's live app (components/home/HeroApp.astro): the visitor picks
// a file and watches the recreated window download it. The add sheet slides
// in with the link, a new row appears at the top of Active and is selected,
// the details pane follows it, the file map fills square by square from
// several connections (the seeded simulation in sim.ts, sped up to a few
// seconds), and it ends Done, then moves to Earlier today. Once after the
// entrance it plays by itself if the visitor has not picked anything.
//
// Under reduced motion nothing plays by itself and a pick shows the finished
// result at once. The loop runs only while the window is on screen and the
// tab is visible. The window is aria-hidden; the chips are the only controls.
import { CHIP_FILES, Fmt, grid, hash, paceOf, ramp, rampPosition, simFor, cells, samplesOf, workers, type Ui } from "./appdemo";
import {
  doneStats,
  icon,
  categoryIcon,
  paneHtml,
  rowHtml,
  runningStats,
  runningStatus,
  sparkHtml,
  statsHtml,
  workersHtml,
  type RowModel,
} from "./appmarkup";
import type { DownloadSim } from "./sim";

type ChipKey = keyof typeof CHIP_FILES;
interface Chip {
  label: string;
  file: string;
  host: string;
  url: string;
}

const root = document.querySelector<HTMLElement>("[data-hero-app]");
if (root) start(root);

function start(root: HTMLElement) {
  const { ui, chips } = JSON.parse(root.dataset.app ?? "{}") as { ui: Ui; chips: Record<ChipKey, Chip> };
  const fmt = new Fmt(ui);
  const reduced = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const $ = <T extends Element = HTMLElement>(sel: string) => root.querySelector<T>(sel);
  const win = $(".win")!;
  const pane = $("[data-pane]")!;
  const activeRows = $("[data-rows='active']")!;
  const todayRows = $("[data-rows='today']")!;
  const sheet = {
    link: $("[data-s='link']")!,
    card: $(".s-card")!,
    type: $("[data-s='type']")!,
    name: $("[data-s='name']")!,
    meta: $("[data-s='meta']")!,
    go: $("[data-s='go']")!,
    add: $("[data-add]")!,
  };
  const counts: Record<string, number> = { all: 11, active: 3, completed: 7, main: 4, today: 7 };
  const times = new Intl.DateTimeFormat("en", { hour: "numeric", minute: "2-digit" });
  const g = grid();

  // Without container query units in trigonometry, scale the window from script.
  if (!CSS.supports("width", "calc(1px * tan(atan2(1px, 2px)))")) {
    const screen = $(".screen")!;
    new ResizeObserver(() => win.style.setProperty("--k", String(screen.clientWidth / 1280))).observe(screen);
  }

  // "Earlier today": times before now that stay within today.
  const sinceMidnight = (() => {
    const d = new Date();
    return d.getHours() * 60 + d.getMinutes();
  })();
  const OFFSETS = [12, 38, 64, 95, 140, 190, 260];
  const squeeze = Math.min(1, Math.max(0, sinceMidnight - 1) / 270);
  todayRows.querySelectorAll<HTMLElement>(".row .rest").forEach((el, i) => {
    const minutes = (OFFSETS[i] ?? 300) * squeeze;
    el.textContent = times.format(new Date(Date.now() - minutes * 60_000));
  });

  const bump = (key: string, by: number) => {
    counts[key] = (counts[key] ?? 0) + by;
    root.querySelectorAll(`[data-n='${key}']`).forEach((el) => (el.textContent = String(counts[key])));
  };

  // The current download, if one is playing.
  interface Run {
    key: ChipKey;
    chip: Chip;
    sim: DownloadSim;
    pace: number;
    row: HTMLElement;
    samples: number[];
    nextSample: number;
    map: MapCanvas | null;
    done: boolean;
  }
  let run: Run | null = null;
  let serial = 0;
  let picked = false;
  let visible = false;
  let frame = 0;
  let last = 0;
  let lastTick = 0;
  let timers: number[] = [];
  const later = (ms: number, fn: () => void) => timers.push(window.setTimeout(fn, ms));

  const rowModel = (r: Run): RowModel => {
    const s = r.sim;
    if (r.done) {
      return { id: r.key, name: r.chip.file, state: "completed", fraction: 1, status: fmt.join(fmt.bytes(s.o.total), r.chip.host), rest: ui.done, restIcon: "check" };
    }
    const frac = s.downloaded / s.o.total;
    return {
      id: r.key,
      name: r.chip.file,
      state: "downloading",
      fraction: frac,
      status: s.downloaded > 0 ? runningStatus(s, fmt) : r.chip.host,
      rest: s.downloaded > 0 ? fmt.percent(frac * 100) : ui.starting,
    };
  };

  const select = (row: HTMLElement) => {
    root.querySelectorAll(".row.selected").forEach((el) => el.classList.remove("selected"));
    row.classList.add("selected");
  };

  const renderPane = (r: Run) => {
    const s = r.sim;
    const finished = times.format(new Date());
    pane.innerHTML = paneHtml(
      {
        name: r.chip.file,
        sub: fmt.join(r.chip.host, fmt.bytes(s.o.total)),
        complete: r.done,
        stats: r.done ? doneStats(s.o.total, finished, ui, fmt) : runningStats(s, ui, fmt),
        note: r.done ? ui.verified : fmt.mapNote(s.active),
        samples: r.samples,
        peak: fmt.peak(r.samples.length ? Math.max(...r.samples) : 0),
        workers: workers(s),
        pathLabel: r.done ? ui.saved : ui.saving,
        path: ui.folder + r.chip.file,
        url: r.chip.url,
        map: '<canvas aria-hidden="true"></canvas>',
      },
      ui,
      fmt,
    );
    const canvas = pane.querySelector("canvas")!;
    const settledAlready = r.map?.settled ?? false;
    r.map = new MapCanvas(canvas, g, r.map);
    if (r.done && (reduced || settledAlready)) r.map.settleNow();
    r.map.draw(s, r.done);
  };

  // Numbers change four times a second, like an engine tick.
  const tick = (r: Run) => {
    const s = r.sim;
    const m = rowModel(r);
    const fill = r.row.querySelector<HTMLElement>(".fill");
    if (fill) fill.style.transform = `scaleX(${m.fraction.toFixed(4)})`;
    r.row.querySelector(".status")!.textContent = m.status;
    r.row.querySelector(".rest")!.textContent = m.rest;
    pane.querySelector("[data-p='stats']")!.innerHTML = statsHtml(runningStats(s, ui, fmt));
    pane.querySelector("[data-p='note']")!.textContent = fmt.mapNote(s.active);
    pane.querySelector("[data-p='peak']")!.textContent = fmt.peak(r.samples.length ? Math.max(...r.samples) : 0);
    pane.querySelector("[data-p='spark']")!.innerHTML = sparkHtml(r.samples);
    pane.querySelector("[data-p='workers']")!.innerHTML = workersHtml(workers(s), fmt);
  };

  const finish = (r: Run, instant: boolean) => {
    r.done = true;
    const id = serial;
    r.row.outerHTML = rowHtml(rowModel(r), true);
    r.row = root.querySelector<HTMLElement>(`[data-row='${r.key}']`)!;
    renderPane(r);
    setPressed(null);
    const move = () => moveToToday(r, !instant && !reduced);
    if (instant) move();
    else later(1600, () => id === serial && move());
  };

  // Done downloads join the top of Earlier today, with the time they finished.
  const moveToToday = (r: Run, animate: boolean) => {
    const model: RowModel = { ...rowModel(r), rest: times.format(new Date()), restIcon: undefined };
    const selected = r.row.classList.contains("selected");
    r.row.remove();
    todayRows.insertAdjacentHTML("afterbegin", rowHtml(model, selected));
    r.row = todayRows.firstElementChild as HTMLElement;
    if (animate) r.row.classList.add("enter");
    bump("active", -1);
    bump("main", -1);
    bump("completed", 1);
    bump("today", 1);
    if (run === r) run = null;
  };

  const setPressed = (key: ChipKey | null) => {
    root.querySelectorAll<HTMLButtonElement>("[data-chip]").forEach((b) => b.setAttribute("aria-pressed", String(b.dataset.chip === key)));
  };

  /** Adds the download's row at the top of Active and selects it. */
  const begin = (key: ChipKey, chip: Chip, instant: boolean) => {
    const file = CHIP_FILES[key];
    const pace = paceOf(file);
    const sim = simFor(file, pace);
    // A file picked again starts over: its old row goes.
    const old = todayRows.querySelector(`[data-row='${key}']`);
    if (old) {
      old.remove();
      bump("all", -1);
      bump("completed", -1);
      bump("today", -1);
    }
    const r: Run = { key, chip, sim, pace, row: activeRows, samples: [], nextSample: 0.5, map: null, done: false };
    activeRows.insertAdjacentHTML("afterbegin", rowHtml(rowModel(r)));
    r.row = activeRows.firstElementChild as HTMLElement;
    if (!instant) r.row.classList.add("enter");
    bump("all", 1);
    bump("active", 1);
    bump("main", 1);
    select(r.row);
    run = r;
    if (instant) {
      sim.advance(1e5);
      r.samples = samplesOf(sim);
      finish(r, true);
      return;
    }
    renderPane(r);
    pane.classList.remove("swap");
    void pane.offsetWidth;
    pane.classList.add("swap");
    lastTick = 0;
    resume();
  };

  const play = (key: ChipKey) => {
    const chip = chips[key];
    picked = true;
    serial += 1;
    const id = serial;
    timers.forEach(clearTimeout);
    timers = [];
    // A download still playing jumps to its end.
    if (run && !run.done) {
      run.sim.advance(1e5);
      finish(run, true);
    } else if (run?.done) {
      // Its move to Earlier today was still pending.
      moveToToday(run, false);
    }
    win.classList.remove("adding");
    setPressed(key);
    if (reduced) {
      begin(key, chip, true);
      setPressed(null);
      return;
    }
    // The add sheet: the link pastes in, the probe finds the file, Download.
    const meta = fmt.join(fmt.join(chip.host, fmt.bytes(CHIP_FILES[key].total)), ui.canResume);
    sheet.link.textContent = "";
    sheet.card.classList.remove("found");
    sheet.type.innerHTML = icon(categoryIcon(chip.file), 20);
    sheet.name.textContent = chip.file;
    sheet.meta.textContent = ui.probing;
    sheet.add.classList.add("press");
    later(140, () => sheet.add.classList.remove("press"));
    later(160, () => win.classList.add("adding"));
    const typed = 18;
    for (let i = 1; i <= typed; i++) {
      later(300 + i * 22, () => (sheet.link.textContent = chip.url.slice(0, Math.round((chip.url.length * i) / typed))));
    }
    later(760, () => sheet.card.classList.add("found"));
    later(1180, () => (sheet.meta.textContent = meta));
    later(1700, () => sheet.go.classList.add("press"));
    later(1860, () => {
      sheet.go.classList.remove("press");
      win.classList.remove("adding");
    });
    later(1980, () => id === serial && begin(key, chip, false));
  };

  // The loop: the simulation advances only while the window is seen.
  const loop = (now: number) => {
    frame = 0;
    const r = run;
    if (!r || r.done || !visible || document.hidden) return;
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    r.sim.advance(dt * r.pace);
    while (r.sim.t >= r.nextSample) {
      r.samples.push(r.sim.speed);
      r.nextSample += 0.5;
    }
    r.map?.draw(r.sim, false);
    if (now - lastTick >= 250) {
      lastTick = now;
      tick(r);
    }
    if (r.sim.finished) {
      finish(r, false);
      return;
    }
    frame = requestAnimationFrame(loop);
  };
  const resume = () => {
    if (frame || !run || run.done || !visible || document.hidden) return;
    last = performance.now();
    frame = requestAnimationFrame(loop);
  };

  for (const button of root.querySelectorAll<HTMLButtonElement>("[data-chip]")) {
    button.setAttribute("aria-pressed", "false");
    button.addEventListener("click", () => play(button.dataset.chip as ChipKey));
  }
  $("[data-try]")!.hidden = false;

  new IntersectionObserver(([entry]) => {
    visible = !!entry?.isIntersecting;
    if (visible) {
      resume();
      run?.map?.wake(run.sim, run.done);
    }
  }).observe(root);
  document.addEventListener("visibilitychange", resume);

  // Once, after the entrance: the first file plays by itself.
  if (!reduced && !document.documentElement.classList.contains("frozen")) {
    const auto = () => {
      if (picked) return;
      if (!visible || document.hidden) {
        window.setTimeout(auto, 600);
        return;
      }
      play("iso");
      picked = true;
    };
    window.setTimeout(auto, 2000);
  }
}

/**
 * The live file map (apps/desktop/src/lib/details/FileMap.svelte): a square
 * eases in as its bytes land and carries a brief light; the square a
 * connection is writing is deep ink with a soft halo; when the download
 * finishes the field settles into one calm tint.
 */
class MapCanvas {
  private arrived: Float64Array;
  private settleStart = NaN;
  private first = true;
  private frame = 0;
  settled = false;

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly g: ReturnType<typeof grid>,
    previous: MapCanvas | null,
  ) {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    canvas.width = Math.round(g.width * dpr);
    canvas.height = Math.round(g.height * dpr);
    this.arrived = previous?.arrived ?? new Float64Array(g.cols * g.rows).fill(NaN);
    this.first = !previous;
  }

  settleNow() {
    this.settleStart = -1e9;
  }

  /** Keeps drawing while squares ease in or the field settles. */
  wake(sim: DownloadSim, complete: boolean) {
    if (this.frame) return;
    const go = () => {
      this.frame = 0;
      if (this.draw(sim, complete)) this.frame = requestAnimationFrame(go);
    };
    this.frame = requestAnimationFrame(go);
  }

  /** Draws one frame; returns true while something is still easing. */
  draw(sim: DownloadSim, complete: boolean): boolean {
    const ctx = this.canvas.getContext("2d");
    if (!ctx) return false;
    const { g } = this;
    const dpr = this.canvas.width / g.width;
    const n = g.cols * g.rows;
    const { fill, head } = cells(sim, n);
    const now = performance.now() / 1000;
    let busy = false;
    if (complete && Number.isNaN(this.settleStart)) this.settleStart = now;
    const settle = complete ? ease((now - this.settleStart) / 0.7) : 0;
    if (complete && settle < 1) busy = true;
    this.settled = complete && settle >= 1;

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, g.width, g.height);
    const band = Math.max(1, Math.round(g.cell * 0.38));
    const depth = Math.max(1, Math.round(g.cell * 0.1));
    for (let i = 0; i < n; i++) {
      const col = i % g.cols;
      const row = Math.floor(i / g.cols);
      const x = g.left + col * (g.cell + g.gap);
      const y = row * (g.cell + g.gap);
      const v = hash(i);
      ctx.globalAlpha = 0.05 * (0.7 + 0.6 * v);
      ctx.fillStyle = "#0A0A0B";
      ctx.fillRect(x, y, g.cell, g.cell);
      const f = fill[i] ?? 0;
      if (f <= 0) {
        this.arrived[i] = NaN;
        continue;
      }
      let [r, gg, b] = ramp(rampPosition(col, row, g.cols, g.rows));
      let alpha: number;
      if (f >= 1) {
        if (Number.isNaN(this.arrived[i])) this.arrived[i] = this.first ? -1e9 : now;
        const age = now - (this.arrived[i] ?? 0);
        if (age < 0.9) busy = true;
        alpha = 0.35 + 0.65 * ease(age / 0.45);
        const flash = Math.exp(-age / 0.32) * 0.55;
        r += (255 - r) * flash;
        gg += (255 - gg) * flash;
        b += (255 - b) * flash;
        alpha += (0.42 + 0.1 * v - alpha) * settle;
      } else {
        alpha = 0.16 + 0.34 * f;
      }
      ctx.globalAlpha = alpha;
      ctx.fillStyle = `rgb(${r | 0},${gg | 0},${b | 0})`;
      ctx.fillRect(x, y, g.cell, g.cell);
      if (f >= 1) {
        ctx.globalAlpha = alpha * 0.16;
        ctx.fillStyle = "#FFFFFF";
        ctx.fillRect(x, y, g.cell, band);
        ctx.fillStyle = "#0A0A0B";
        ctx.fillRect(x, y + g.cell - depth, g.cell, depth);
      }
      if (!complete && head[i]) {
        ctx.globalAlpha = 0.24;
        ctx.fillStyle = "#0A0A0B";
        ctx.fillRect(x - g.gap / 2, y - g.gap / 2, g.cell + g.gap, g.cell + g.gap);
        ctx.globalAlpha = 1;
        ctx.fillStyle = "#0A0A0B";
        ctx.fillRect(x, y, g.cell, g.cell);
      }
    }
    ctx.globalAlpha = 1;
    this.first = false;
    if (complete && busy) this.wake(sim, complete);
    return busy;
  }
}

const ease = (x: number) => 1 - Math.pow(1 - Math.max(0, Math.min(1, x)), 3);
