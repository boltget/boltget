<!--
  The home page hero (docs/WEBSITE.md, Hero concept).

  1. The BoltGet arrow at hero scale lights up row by row. This is CSS, so it
     also plays without JavaScript; under reduced motion it is fully lit.
  2. Its pixels flow into a progress bar, which plays a simulated segmented
     download: segments fill at different rates; when one finishes, the
     largest remaining segment splits in two and the free connection takes
     the second half. The connection count adapts from 4 upwards.
  3. The numbers are a simulation and are labelled as such, linking to the
     real benchmarks.

  Randomness is seeded and the simulation runs on a fixed time step, so every
  visitor sees the same sequence. The loop pauses while the hero is off
  screen or the tab is hidden. Without JavaScript, or with reduced motion, a
  static completed segmented bar is shown instead.

  The live layer is drawn imperatively (a few pooled SVG rects updated per
  frame) rather than with reactive blocks: it keeps the island's JavaScript
  small and avoids per-frame diffing.
-->
<script lang="ts" module>
  // ---- Geometry (SVG user units) -------------------------------------------
  // The mark's grid: 9 x 9 cells, square pixels at 10/12 of the pitch.
  const W = 720;
  const PITCH = 42;
  const PIXEL = 35;
  const ARROW = 8 * PITCH + PIXEL;
  const AX = (W - ARROW) / 2;
  const BAR_Y = ARROW + 64;
  const BAR_H = 28;
  const H = BAR_Y + BAR_H;
  const SLOT_GAP = 4;

  // Rows of the mark (brand/BRAND.md, Construction): [first col, last col].
  const ROWS: [number, number][] = [
    [3, 5], [3, 5], [3, 5], [3, 5], [0, 8], [1, 7], [2, 6], [3, 5], [4, 4],
  ];
  const pixels = ROWS.flatMap(([from, to], row) =>
    Array.from({ length: to - from + 1 }, (_, i) => ({
      row,
      col: from + i,
      x: AX + (from + i) * PITCH,
      y: row * PITCH,
    })),
  );
  const square = (x: number, y: number) => `M${x} ${y}h${PIXEL}v${PIXEL}h-${PIXEL}z`;
  const arrowMarkup = ROWS.map(
    (_, row) =>
      `<path class="row" style="--row:${row}" d="${pixels
        .filter((p) => p.row === row)
        .map((p) => square(p.x, p.y))
        .join("")}"/>`,
  ).join("");

  // A finished download for the static state: segments of uneven sizes, as
  // dynamic splitting leaves them.
  const staticMarkup = [0.21, 0.37, 0.5, 0.66, 0.83]
    .map((at) => `<rect class="sep" x="${at * W - 1.5}" y="${BAR_Y}" width="3" height="${BAR_H}"/>`)
    .join("");

  // Where each pixel lands: one slot per pixel across the bar, in column order.
  const slotW = (W - (pixels.length - 1) * SLOT_GAP) / pixels.length;
  const flights = [...pixels]
    .sort((a, b) => a.col - b.col || a.row - b.row)
    .map((p, slot) => ({
      ...p,
      tx: slot * (slotW + SLOT_GAP),
      delay: (8 - p.row) * 0.045 + Math.abs(p.col - 4) * 0.012,
    }));

  // ---- Timing (seconds) ------------------------------------------------------
  const ARRIVE_END = 8 * 0.12 + 0.31; // last row lit (brand motion: 0.12 s stagger, 0.31 s rise)
  const HOLD = 0.3;
  const FLIGHT = 0.55;
  const FLIGHT_END = Math.max(...flights.map((f) => f.delay)) + FLIGHT;
  const MERGE = 0.3;
  const DONE_HOLD = 2.4;
  const RESET = 0.45;
  const STEP = 1 / 120;

  // ---- Simulation ------------------------------------------------------------
  const TOTAL = 320; // MiB
  const MIN_SPLIT = 8; // MiB: a segment with less than twice this left is not split
  const START_CONNECTIONS = 4;
  const MAX_CONNECTIONS = 6;
  const ADD_EVERY = 0.9; // s: the adaptive controller adds a connection
  const POOL = 16;

  function mulberry32(seed: number) {
    let a = seed >>> 0;
    return () => {
      a = (a + 0x6d2b79f5) >>> 0;
      let t = a;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  interface Seg {
    start: number;
    end: number;
    pos: number;
    born: number;
    base: number;
    freq: number;
    phase: number;
    done: boolean;
  }

  interface Sim {
    t: number;
    segs: Seg[];
    marks: { at: number; born: number }[];
    nextAdd: number;
    finished: boolean;
  }

  const clamp01 = (x: number) => Math.min(1, Math.max(0, x));
  const lerp = (a: number, b: number, k: number) => a + (b - a) * k;
  const ease = (x: number) => (x < 0.5 ? 4 * x * x * x : 1 - (-2 * x + 2) ** 3 / 2);
  const sx = (mib: number) => (mib / TOTAL) * W;
</script>

<script lang="ts">
  import { onMount } from "svelte";

  interface Labels {
    title: string;
    honesty: string;
    honestyLink: string;
    honestyHref: string;
    staticLabel: string;
    connections: { one: string; other: string };
    progress: string;
    speed: string;
    done: string;
  }

  let { labels }: { labels: Labels } = $props();

  let root: HTMLElement;
  let live: SVGGElement;
  let flight: SVGGElement;
  let stats: HTMLElement;
  let connText: HTMLElement;
  let progressText: HTMLElement;
  let speedText: HTMLElement;

  onMount(() => {
    const NS = "http://www.w3.org/2000/svg";
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
    const plural = new Intl.PluralRules("en");
    const fill = (msg: string, params: Record<string, string | number>) =>
      msg.replace(/\{(\w+)\}/g, (_, k: string) => String(params[k] ?? ""));

    const rand = mulberry32(0xb0176e7);
    const between = (lo: number, hi: number) => lo + (hi - lo) * rand();

    // -- Drawing pools --------------------------------------------------------
    function rect(parent: Element, cls: string, attrs: Record<string, string | number> = {}) {
      const r = document.createElementNS(NS, "rect");
      if (cls) r.setAttribute("class", cls);
      for (const [k, v] of Object.entries(attrs)) r.setAttribute(k, String(v));
      parent.appendChild(r);
      return r;
    }
    const track = rect(live, "track", { x: 0, y: BAR_Y, width: W, height: BAR_H, opacity: 0 });
    const pool = (cls: string, attrs: Record<string, string | number>) =>
      Array.from({ length: POOL }, () => rect(live, cls, attrs));
    const fills = pool("fill", { y: BAR_Y, height: BAR_H });
    const seps = pool("sep", { y: BAR_Y, width: 3, height: BAR_H });
    const heads = pool("head", { y: BAR_Y, width: 2, height: BAR_H });
    const ticks = pool("tick", { y: BAR_Y - 12, width: 3, height: 7 });
    const flyers = flights.map(() => rect(flight, ""));

    const r2 = (n: number) => (Math.round(n * 100) / 100).toString();
    function place(list: SVGRectElement[], values: { x: number; w?: number; o?: number }[]) {
      list.forEach((el, i) => {
        const v = values[i];
        if (v === undefined) {
          el.setAttribute("display", "none");
          return;
        }
        el.removeAttribute("display");
        el.setAttribute("x", r2(v.x));
        if (v.w !== undefined) el.setAttribute("width", r2(Math.max(0, v.w)));
        if (v.o !== undefined) el.setAttribute("opacity", r2(v.o));
      });
    }

    // -- Simulation -----------------------------------------------------------
    function connection(start: number, end: number, born: number): Seg {
      return {
        start,
        end,
        pos: start,
        born,
        base: between(10.5, 15.5),
        freq: between(1.6, 3.4),
        phase: between(0, Math.PI * 2),
        done: false,
      };
    }

    function newSim(): Sim {
      const segs = Array.from({ length: START_CONNECTIONS }, (_, i) =>
        connection((TOTAL * i) / START_CONNECTIONS, (TOTAL * (i + 1)) / START_CONNECTIONS, 0),
      );
      return { t: 0, segs, marks: [], nextAdd: ADD_EVERY, finished: false };
    }

    const rate = (s: Seg, t: number) =>
      s.base * (1 + 0.2 * Math.sin(s.freq * t + s.phase)) * Math.min(1, 0.15 + (t - s.born) / 0.35);
    const activeCount = (s: Sim) => s.segs.filter((seg) => !seg.done).length;

    /** Splits the largest remaining segment; a new connection takes its second half. */
    function split(s: Sim) {
      let largest: Seg | undefined;
      for (const seg of s.segs) {
        if (!seg.done && (!largest || seg.end - seg.pos > largest.end - largest.pos)) largest = seg;
      }
      if (!largest || largest.end - largest.pos < MIN_SPLIT * 2) return;
      const mid = largest.pos + (largest.end - largest.pos) / 2;
      s.segs.push(connection(mid, largest.end, s.t));
      largest.end = mid;
      s.marks.push({ at: mid, born: s.t });
    }

    function step(s: Sim) {
      s.t += STEP;
      if (s.t >= s.nextAdd) {
        s.nextAdd += ADD_EVERY;
        if (activeCount(s) < MAX_CONNECTIONS) split(s);
      }
      for (const seg of s.segs) {
        if (seg.done) continue;
        seg.pos = Math.min(seg.end, seg.pos + rate(seg, s.t) * STEP);
        if (seg.pos >= seg.end) {
          seg.done = true;
          split(s);
        }
      }
      s.finished = s.segs.every((seg) => seg.done);
    }

    // -- Rendering ------------------------------------------------------------
    let shownSpeed = 0;
    let lastText = -1;

    function drawBar(s: Sim, now: number) {
      const sorted = [...s.segs].sort((a, b) => a.start - b.start);
      place(fills, sorted.map((seg) => ({ x: sx(seg.start), w: sx(seg.pos) - sx(seg.start) })));
      place(seps, sorted.filter((seg) => seg.start > 0).map((seg) => ({ x: sx(seg.start) - 1.5 })));
      place(heads, sorted.filter((seg) => !seg.done).map((seg) => ({ x: sx(seg.pos) - 2 })));
      place(
        ticks,
        s.marks
          .map((m) => ({ x: sx(m.at) - 1.5, o: 1 - (s.t - m.born) / 0.8 }))
          .filter((m) => m.o > 0),
      );

      const speed = sorted.reduce((sum, seg) => sum + (seg.done ? 0 : rate(seg, s.t)), 0);
      shownSpeed = shownSpeed === 0 ? speed : shownSpeed + (speed - shownSpeed) * 0.25;
      if (now - lastText < 0.12 && !s.finished) return;
      lastText = now;
      const count = activeCount(s);
      const forms = labels.connections;
      connText.textContent = s.finished
        ? ""
        : fill(plural.select(count) === "one" ? forms.one : forms.other, { count });
      const done = sorted.reduce((sum, seg) => sum + (seg.pos - seg.start), 0);
      progressText.textContent = fill(labels.progress, { done: Math.round(done), total: TOTAL });
      speedText.textContent = s.finished ? labels.done : fill(labels.speed, { value: shownSpeed.toFixed(1) });
    }

    function drawFlight(t: number) {
      flyers.forEach((el, i) => {
        const f = flights[i]!;
        const k = ease(clamp01((t - f.delay) / FLIGHT));
        el.setAttribute("x", r2(lerp(f.x, f.tx, k)));
        el.setAttribute("y", r2(lerp(f.y, BAR_Y, k)));
        el.setAttribute("width", r2(lerp(PIXEL, slotW, k)));
        el.setAttribute("height", r2(lerp(PIXEL, BAR_H, k)));
      });
      flight.setAttribute("opacity", r2(1 - clamp01((t - FLIGHT_END) / MERGE)));
      track.setAttribute("opacity", r2(clamp01((t - FLIGHT_END + MERGE * 0.5) / MERGE)));
    }

    // -- Loop -----------------------------------------------------------------
    type Phase = "ssr" | "static" | "arrive" | "flow" | "run";
    let phase: Phase = "ssr";
    let sim = newSim();
    let clock = 0;
    let doneAt = 0;
    let acc = 0;
    let startDelay = 0;
    let raf = 0;
    let last = 0;
    let onScreen = true;

    function setPhase(next: Phase) {
      phase = next;
      root.dataset.phase = next;
    }

    const running = () => onScreen && document.visibilityState === "visible" && !reduced.matches;

    function frame(nowMs: number) {
      raf = 0;
      if (!running()) return;
      const now = nowMs / 1000;
      const dt = last === 0 ? 0 : Math.min(0.1, now - last);
      last = now;

      if (phase === "arrive") {
        startDelay -= dt;
        if (startDelay <= 0) {
          setPhase("flow");
          clock = 0;
        }
      } else if (phase === "flow") {
        clock += dt;
        drawFlight(clock);
        if (clock >= FLIGHT_END + MERGE) {
          setPhase("run");
          track.setAttribute("opacity", "1");
          acc = 0;
        }
      } else if (phase === "run") {
        clock += dt;
        if (!sim.finished) {
          acc += dt;
          while (acc >= STEP && !sim.finished) {
            step(sim);
            acc -= STEP;
          }
          if (sim.finished) doneAt = clock;
          live.setAttribute("opacity", "1");
          stats.style.opacity = "1";
        } else {
          const since = clock - doneAt;
          const o = since < DONE_HOLD ? 1 : 1 - clamp01((since - DONE_HOLD) / RESET);
          live.setAttribute("opacity", r2(o));
          stats.style.opacity = r2(o);
          if (since >= DONE_HOLD + RESET) {
            sim = newSim();
            acc = 0;
            shownSpeed = 0;
          }
        }
        drawBar(sim, clock);
      }
      schedule();
    }

    function schedule() {
      if (raf === 0 && running()) raf = requestAnimationFrame(frame);
    }

    function wake() {
      last = 0;
      schedule();
    }

    function applyMotion() {
      if (reduced.matches) {
        cancelAnimationFrame(raf);
        raf = 0;
        setPhase("static");
        return;
      }
      if (phase === "static") {
        setPhase("run");
      } else if (phase === "ssr") {
        // The arrow's CSS animation started with the first paint: start the
        // flow once its last row is lit, or at once if that already happened.
        const paint = performance.getEntriesByName("first-contentful-paint")[0];
        const sincePaint = (performance.now() - (paint?.startTime ?? 0)) / 1000;
        startDelay = Math.max(0, ARRIVE_END + HOLD - sincePaint);
        setPhase("arrive");
      }
      wake();
    }

    const io = new IntersectionObserver((entries) => {
      onScreen = entries.some((e) => e.isIntersecting);
      wake();
    });
    io.observe(root);
    document.addEventListener("visibilitychange", wake);
    reduced.addEventListener("change", applyMotion);
    applyMotion();

    return () => {
      cancelAnimationFrame(raf);
      io.disconnect();
      document.removeEventListener("visibilitychange", wake);
      reduced.removeEventListener("change", applyMotion);
    };
  });
</script>

<div class="stage" bind:this={root} data-phase="ssr">
  <svg viewBox="0 0 {W} {H}" role="img" aria-labelledby="hero-stage-title">
    <title id="hero-stage-title">{labels.title}</title>
    <defs>
      <linearGradient id="hero-mark" gradientUnits="userSpaceOnUse" x1={AX} y1="0" x2={AX + ARROW} y2={ARROW}>
        <stop offset="0" stop-color="#A7F3D0" />
        <stop offset="0.5" stop-color="#34D399" />
        <stop offset="1" stop-color="#059669" />
      </linearGradient>
      <linearGradient id="hero-bar" gradientUnits="userSpaceOnUse" x1="0" y1="0" x2={W} y2="0">
        <stop offset="0" stop-color="#A7F3D0" />
        <stop offset="0.5" stop-color="#34D399" />
        <stop offset="1" stop-color="#059669" />
      </linearGradient>
      <radialGradient id="hero-glow" cx="0.5" cy="0.5" r="0.5">
        <stop offset="0" stop-color="#34D399" stop-opacity="0.11" />
        <stop offset="1" stop-color="#34D399" stop-opacity="0" />
      </radialGradient>
    </defs>
    <ellipse cx={W / 2} cy={ARROW * 0.55} rx={ARROW * 0.95} ry={ARROW * 0.8} fill="url(#hero-glow)" />
    <g class="arrow" fill="url(#hero-mark)">{@html arrowMarkup}</g>
    <g class="static" data-hero-static>
      <rect x="0" y={BAR_Y} width={W} height={BAR_H} fill="url(#hero-bar)" />
      {@html staticMarkup}
    </g>
    <g class="live" bind:this={live}></g>
    <g class="flight" fill="url(#hero-mark)" bind:this={flight}></g>
  </svg>

  <div class="readout">
    <p class="stats num" aria-hidden="true" bind:this={stats}>
      <span bind:this={connText}></span>
      <span bind:this={progressText}></span>
      <span class="speed" bind:this={speedText}></span>
    </p>
    <p class="static-label" data-hero-static>{labels.staticLabel}</p>
    <p class="honesty">
      {labels.honesty} <a href={labels.honestyHref}>{labels.honestyLink}</a>
    </p>
  </div>
</div>

<style>
  .stage {
    width: 100%;
  }
  svg {
    width: 100%;
    height: auto;
    aspect-ratio: 720 / 463;
    overflow: visible;
  }
  .arrow :global(.row) {
    opacity: 0.15;
    animation: arrive 0.31s linear calc(var(--row) * 0.12s) both;
  }
  @keyframes arrive {
    from {
      opacity: 0.15;
    }
    to {
      opacity: 1;
    }
  }
  svg :global(.track) {
    fill: var(--raised);
  }
  svg :global(.fill) {
    fill: url(#hero-bar);
  }
  svg :global(.sep) {
    fill: var(--bg);
  }
  svg :global(.head) {
    fill: var(--emerald-50);
  }
  svg :global(.tick) {
    fill: var(--emerald-200);
  }
  .live,
  .flight {
    display: none;
  }
  .stage:global([data-phase="flow"]) .flight,
  .stage:global([data-phase="flow"]) .live,
  .stage:global([data-phase="run"]) .live {
    display: inline;
  }
  .readout {
    display: grid;
    gap: 0.5rem;
    margin-top: 0.875rem;
    font-size: 0.8125rem;
    line-height: 1.5;
  }
  .stats,
  .static-label {
    grid-area: 1 / 1;
    min-height: 1.25rem;
    margin: 0;
  }
  .stats {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    color: var(--text-muted);
    white-space: nowrap;
    opacity: 0;
  }
  .speed {
    color: var(--text);
    min-width: 7ch;
    text-align: right;
  }
  .static-label {
    color: var(--text-muted);
    text-align: center;
  }
  .honesty {
    margin: 0;
    color: var(--text-subtle);
    text-align: center;
  }
  .honesty a {
    color: var(--text-muted);
  }
  .honesty a:hover {
    color: var(--accent);
  }

  /* The static bar shows without JavaScript (see the page's <noscript>) and
     under reduced motion; with motion and JavaScript the live bar replaces it. */
  @media (prefers-reduced-motion: no-preference) {
    [data-hero-static] {
      visibility: hidden;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .arrow :global(.row) {
      animation: none;
      opacity: 1;
    }
    .stats {
      visibility: hidden;
    }
  }
  @media (max-width: 30rem) {
    .stats {
      font-size: 0.75rem;
      gap: 0.5rem;
    }
  }
</style>
