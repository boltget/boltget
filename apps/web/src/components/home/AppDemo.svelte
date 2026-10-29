<!--
  Home hero centerpiece: a simulated preview of the planned desktop app (v0.2).
  A link is typed and sent; the file is probed; four connections start and the
  adaptive controller adds more while they help; connections that finish early
  take over half of the largest part left; the file map fills square by square;
  the download lands with a verified checksum. Then it loops.

  Honesty: the card carries a "Preview · v0.2 app" chip and the caption says
  the run is simulated with illustrative numbers, linking to the benchmarks.

  Server render and reduced motion show the finished state as a still. The
  loop pauses off screen and in hidden tabs.
-->
<script lang="ts" module>
  import { DEMO_SIM, DownloadSim, type SimOptions } from "../../lib/sim";
  import { ChunkMap } from "../../lib/chunkmap";

  export const SIM: SimOptions = DEMO_SIM;

  function finished(): DownloadSim {
    const s = new DownloadSim(SIM);
    while (!s.finished) s.step();
    return s;
  }
  const FINAL = finished();
  const SAMPLES = Math.ceil(FINAL.finishedAt * 10) + 2;
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";
  import iconUrl from "../../../../../brand/svg/icon-16.svg?url";

  interface Strings {
    preview: string;
    appName: string;
    nav: { all: string; active: string; completed: string; scheduled: string };
    empty: string;
    download: string;
    connecting: string;
    meta: string;
    stats: { speed: string; received: string; left: string; connections: string };
    map: string;
    mapNote: string;
    graph: string;
    conn: { connecting: string; receiving: string; idle: string };
    done: string;
    verified: string;
    open: string;
    label: string;
    note: string;
    noteLink: string;
    url: string;
    name: string;
    host: string;
    recent: { heading: string; done: string; items: { name: string; size: string }[] };
  }

  let { s }: { s: Strings } = $props();

  type Phase = "idle" | "typing" | "pressed" | "probing" | "downloading" | "done";

  interface ConnRow {
    id: number;
    state: "connecting" | "receiving" | "idle";
    rate: number;
    note: string;
  }

  interface View {
    phase: Phase;
    typed: number;
    speed: number;
    received: number;
    eta: number | null;
    active: number;
    elapsed: number;
    history: number[];
    conns: ConnRow[];
    fading: boolean;
  }

  const fill = (msg: string, p: Record<string, string | number>) =>
    msg.replace(/\{(\w+)\}/g, (_, k: string) => String(p[k] ?? ""));

  function viewOf(sim: DownloadSim, phase: Phase, typed: number, fading = false): View {
    return {
      phase,
      typed,
      speed: sim.speed,
      received: sim.downloaded,
      eta: sim.eta(),
      active: sim.active,
      elapsed: sim.finished ? sim.finishedAt : sim.t,
      history: sim.history.slice(),
      conns: sim.conns.map((c) => ({
        id: c.id,
        state: sim.finished ? "idle" : c.state,
        rate: c.rate,
        note: sim.t - c.noteAt < 1.4 ? c.note : "",
      })),
      fading,
    };
  }

  // The still: the finished download.
  let view = $state.raw<View>(viewOf(FINAL, "done", 999));
  let canvas: HTMLCanvasElement;
  let root: HTMLElement;

  const T_TYPE = 0.7;
  const CHAR = 0.03;

  const phaseRank: Record<Phase, number> = { idle: 0, typing: 1, pressed: 2, probing: 3, downloading: 4, done: 5 };
  const atLeast = (p: Phase) => phaseRank[view.phase] >= phaseRank[p];

  const pct = $derived(Math.floor((view.received / SIM.total) * 100));
  const graph = $derived.by(() => {
    const W = 320;
    const H = 84;
    const max = 72;
    const pts = view.history.map((v, i) => [(i / (SAMPLES - 1)) * W, H - (v / max) * (H - 6)] as const);
    if (pts.length < 2) return { line: "", area: "", end: null as null | readonly [number, number] };
    const line = pts.map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)} ${y.toFixed(1)}`).join("");
    const last = pts[pts.length - 1]!;
    return { line, area: `${line}L${last[0].toFixed(1)} ${H}L0 ${H}Z`, end: last };
  });

  // ---- File map (canvas, drawn imperatively by lib/chunkmap.ts) --------------
  const ROWS = 6;
  let current: DownloadSim = FINAL;
  let map: ChunkMap | null = null;

  function drawMap(sim: DownloadSim) {
    if (!canvas) return;
    const narrow = canvas.clientWidth < 520;
    const cols = narrow ? 28 : 56;
    const gap = narrow ? 3 : 4;
    if (!map) map = new ChunkMap(canvas, { cols, rows: ROWS, gap, order: "row" });
    if (map.layout.cols !== cols) {
      map.layout = { ...map.layout, cols, gap };
      map.reset();
    }
    map.measure();
    map.draw(sim);
  }

  onMount(() => {
    drawMap(FINAL);
    const ro = new ResizeObserver(() => drawMap(current));
    ro.observe(canvas);

    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
    if (reduced.matches) return () => ro.disconnect();

    let sim = new DownloadSim(SIM);
    current = sim;
    let clock = 0;
    let doneAt = 0;
    let raf = 0;
    let last = 0;
    let lastView = 0;
    let visible = true;
    const typeEnd = T_TYPE + s.url.length * CHAR;
    const press = typeEnd + 0.4;
    const probeEnd = press + 0.8;

    view = viewOf(sim, "idle", 0);
    drawMap(sim);

    const update = (now: number, force = false) => {
      let phase: Phase;
      if (clock < T_TYPE) phase = "idle";
      else if (clock < typeEnd) phase = "typing";
      else if (clock < press + 0.2) phase = "pressed";
      else if (clock < probeEnd) phase = "probing";
      else if (!sim.finished) phase = "downloading";
      else phase = "done";

      if (phase === "downloading") {
        const target = clock - probeEnd;
        while (!sim.finished && sim.t < target) sim.step();
        if (sim.finished) {
          doneAt = clock;
          phase = "done";
        }
      }
      let fading = false;
      if (phase === "done") {
        if (doneAt === 0) doneAt = clock;
        if (clock - doneAt > 4.2) fading = true;
        if (clock - doneAt > 4.8) {
          sim = new DownloadSim(SIM);
          current = sim;
          map?.reset();
          clock = 0;
          doneAt = 0;
          phase = "idle";
        }
      }

      if (force || now - lastView > 33 || phase !== view.phase || fading !== view.fading) {
        lastView = now;
        const typed = phase === "typing" ? Math.floor((clock - T_TYPE) / CHAR) : phase === "idle" ? 0 : s.url.length;
        view = viewOf(sim, phase, typed, fading);
      }
      drawMap(sim);
    };

    // Review aid: ?at=<seconds> freezes the demo at that moment of the loop.
    const at = Number(new URLSearchParams(location.search).get("at"));
    if (at > 0) {
      clock = at;
      update(performance.now(), true);
      // A still: keep drawing until the last squares have eased in.
      const settle = () => {
        drawMap(sim);
        if (map?.busy) requestAnimationFrame(settle);
      };
      requestAnimationFrame(settle);
      return () => ro.disconnect();
    }

    const frame = (now: number) => {
      raf = 0;
      clock += Math.min(0.25, (now - last) / 1000);
      last = now;
      update(now);
      if (visible && !document.hidden) raf = requestAnimationFrame(frame);
    };
    const schedule = () => {
      if (raf || !visible || document.hidden) return;
      last = performance.now();
      raf = requestAnimationFrame(frame);
    };
    const io = new IntersectionObserver(([e]) => {
      visible = e?.isIntersecting ?? true;
      schedule();
    });
    io.observe(root);
    document.addEventListener("visibilitychange", schedule);
    schedule();
    return () => {
      cancelAnimationFrame(raf);
      io.disconnect();
      ro.disconnect();
      document.removeEventListener("visibilitychange", schedule);
    };
  });

  const mib = (v: number) => Math.round(v).toString();
  const rate = (v: number) => v.toFixed(1);
</script>

<figure class="demo" bind:this={root} aria-label={s.label}>
  <div class="window" aria-hidden="true">
    <div class="titlebar">
      <img src={iconUrl} width="16" height="16" alt="" />
      <span class="app">{s.appName}</span>
      <span class="chip">{s.preview}</span>
      <span class="controls">
        <svg viewBox="0 0 10 10" width="10" height="10"><path d="M0 5.5h10" /></svg>
        <svg viewBox="0 0 10 10" width="10" height="10"><path d="M.5.5h9v9h-9z" /></svg>
        <svg viewBox="0 0 10 10" width="10" height="10"><path d="M0 0l10 10M10 0L0 10" /></svg>
      </span>
    </div>
    <div class="body">
      <nav class="side">
        <span class="nav on">{s.nav.all}<b>12</b></span>
        <span class="nav">{s.nav.active}<b>{atLeast("probing") && view.phase !== "done" ? 1 : 0}</b></span>
        <span class="nav" class:bump={view.phase === "done"}>{s.nav.completed}<b>{view.phase === "done" ? 12 : 11}</b></span>
        <span class="nav">{s.nav.scheduled}<b>2</b></span>
      </nav>
      <div class="main">
        <div class="urlbar" class:focus={view.phase === "typing" || view.phase === "idle"}>
          <svg class="link" viewBox="0 0 16 16" width="16" height="16"
            ><path d="M6.5 9.5l3-3M7 4.5l1.2-1.2a2.6 2.6 0 013.7 3.7L10.7 8.2M9 11.5l-1.2 1.2a2.6 2.6 0 01-3.7-3.7L5.3 7.8" /></svg
          >
          <span class="url">
            {#if view.typed > 0}
              <span class="typed">{s.url.slice(0, view.typed)}</span>
            {:else}
              <span class="placeholder">{s.empty}</span>
            {/if}
            {#if view.phase === "idle" || view.phase === "typing"}<span class="caret"></span>{/if}
          </span>
          <span class="go" class:pressed={view.phase === "pressed"}>{s.download}</span>
        </div>

        <div class="job" class:show={atLeast("probing")} class:fading={view.fading}>
          <div class="jobhead">
            <svg class="file" viewBox="0 0 28 34" width="28" height="34"
              ><path d="M1 1h18l8 8v24H1z" /><path d="M19 1v8h8" /><rect x="7" y="20" width="4" height="4" /><rect
                x="12"
                y="20"
                width="4"
                height="4"
              /><rect x="17" y="20" width="4" height="4" /><rect x="12" y="25" width="4" height="4" /></svg
            >
            <div class="who">
              <div class="name">{s.name}</div>
              <div class="meta">
                {view.phase === "probing"
                  ? fill(s.connecting, { host: s.host })
                  : fill(s.meta, { size: `${SIM.total} MiB`, host: s.host })}
              </div>
            </div>
            <div class="pct">{view.phase === "probing" ? "" : `${pct}%`}</div>
          </div>

          <dl class="stats">
            <div>
              <dt>{s.stats.speed}</dt>
              <dd><span class="big">{view.phase === "done" ? "–" : rate(view.speed)}</span> MiB/s</dd>
            </div>
            <div>
              <dt>{s.stats.received}</dt>
              <dd><span class="big">{mib(view.received)}</span> / {SIM.total} MiB</dd>
            </div>
            <div>
              <dt>{s.stats.left}</dt>
              <dd><span class="big">{view.phase === "done" ? "0" : view.eta === null ? "–" : Math.ceil(view.eta)}</span> s</dd>
            </div>
            <div>
              <dt>{s.stats.connections}</dt>
              <dd><span class="big">{view.phase === "done" ? FINAL.peakConnections : view.active}</span></dd>
            </div>
          </dl>

          <div class="mapwrap">
            <div class="label"><span>{s.map}</span><span class="note">{s.mapNote}</span></div>
            <canvas bind:this={canvas}></canvas>
          </div>

          <div class="lower">
            <div class="graph">
              <div class="label"><span>{s.graph}</span></div>
              <svg viewBox="0 0 320 84" preserveAspectRatio="none">
                <path class="grid" d="M0 21h320M0 42h320M0 63h320" />
                {#if graph.end}
                  <path class="area" d={graph.area} />
                  <path class="line" d={graph.line} />
                {/if}
              </svg>
            </div>
            <ul class="conns">
              {#each view.conns as c (c.id)}
                <li in:fly={{ y: 6, duration: 320 }}>
                  <span class="dot" class:off={c.state === "idle"}></span>
                  <span class="cid">#{c.id + 1}</span>
                  <span class="state">{c.note || s.conn[c.state]}</span>
                  <span class="rbar"><span style:width={`${Math.min(100, (c.rate / 14) * 100)}%`}></span></span>
                  <span class="rate">{c.state === "idle" ? "" : rate(c.rate)}</span>
                </li>
              {/each}
            </ul>
          </div>

          <div class="done" class:show={view.phase === "done"}>
            <svg viewBox="0 0 20 20" width="20" height="20"><path d="M4.5 10.5l3.5 3.5 7.5-8" /></svg>
            <span class="what">{fill(s.done, { size: `${SIM.total} MiB`, time: view.elapsed.toFixed(1) })}</span>
            <span class="ok">{s.verified}</span>
            <span class="open">{s.open}</span>
          </div>
        </div>

        <div class="recent" class:pushed={atLeast("probing")}>
          <div class="label"><span>{s.recent.heading}</span></div>
          <ul>
            {#each s.recent.items as r (r.name)}
              <li>
                <span class="rname">{r.name}</span>
                <span class="rsize">{r.size}</span>
                <span class="rdone">{s.recent.done}</span>
              </li>
            {/each}
          </ul>
        </div>
      </div>
    </div>
  </div>
  <figcaption>{s.note} <a class="tlink" href="/benchmarks/">{s.noteLink}</a></figcaption>
</figure>

<style>
  .demo {
    --ink: #0b0b0f;
    --muted: #52525b;
    --subtle: #71717a;
    --line: #e7eae9;
    --soft: #f5f7f6;
    margin: 0;
    color: var(--ink);
  }
  .window {
    overflow: hidden;
    border-radius: 20px;
    background: #fff;
    box-shadow:
      0 0 0 1px rgb(2 44 34 / 0.06),
      0 24px 60px -20px rgb(2 44 34 / 0.35),
      0 60px 120px -40px rgb(2 44 34 / 0.25);
  }
  .titlebar {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    height: 44px;
    padding: 0 0 0 1rem;
    border-bottom: 1px solid var(--line);
    font-size: 0.8125rem;
  }
  .app {
    font-weight: 600;
  }
  .chip {
    padding: 0.15rem 0.55rem;
    border-radius: 999px;
    background: #ecfdf5;
    color: #065f46;
    font: 400 0.6875rem/1.4 var(--font-mono);
    letter-spacing: 0.02em;
  }
  .controls {
    display: flex;
    margin-left: auto;
    height: 100%;
  }
  .controls svg {
    box-sizing: content-box;
    padding: 0 18px;
    height: 100%;
    fill: none;
    stroke: var(--muted);
    stroke-width: 1;
  }
  .body {
    display: grid;
    grid-template-columns: 13.5rem minmax(0, 1fr);
    min-height: 37rem;
  }
  .side {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    padding: 1rem 0.75rem;
    border-right: 1px solid var(--line);
    background: var(--soft);
    font-size: 0.8125rem;
    color: var(--muted);
  }
  .nav {
    display: flex;
    justify-content: space-between;
    padding: 0.45rem 0.65rem;
    border-radius: 8px;
  }
  .nav b {
    font: 400 0.75rem/1.6 var(--font-mono);
    color: var(--subtle);
  }
  .nav.on {
    background: #fff;
    color: var(--ink);
    box-shadow: 0 0 0 1px var(--line);
  }
  .nav.bump b {
    color: #047857;
  }
  .main {
    padding: 1.25rem 1.5rem 1.5rem;
    min-width: 0;
  }
  .urlbar {
    display: flex;
    align-items: center;
    gap: 0.65rem;
    height: 46px;
    padding: 0 0.35rem 0 0.85rem;
    border-radius: 12px;
    border: 1px solid var(--line);
    background: #fff;
    transition: border-color 0.3s, box-shadow 0.3s;
  }
  .urlbar.focus {
    border-color: #a7f3d0;
    box-shadow: 0 0 0 3px rgb(52 211 153 / 0.14);
  }
  .link {
    flex: none;
    fill: none;
    stroke: var(--subtle);
    stroke-width: 1.4;
    stroke-linecap: round;
  }
  .url {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font: 400 0.8125rem/1 var(--font-mono);
  }
  .placeholder {
    color: var(--subtle);
    font-family: var(--font-sans);
    font-size: 0.875rem;
  }
  .caret {
    display: inline-block;
    width: 1.5px;
    height: 1.05em;
    margin-left: 1px;
    vertical-align: -0.2em;
    background: var(--ink);
    animation: blink 1s steps(1) infinite;
  }
  @keyframes blink {
    50% {
      opacity: 0;
    }
  }
  .go {
    flex: none;
    padding: 0.55rem 0.95rem;
    border-radius: 9px;
    background: var(--ink);
    color: #fff;
    font-size: 0.8125rem;
    font-weight: 600;
    transition: transform 0.15s, background 0.15s;
  }
  .go.pressed {
    transform: scale(0.96);
    background: #047857;
  }

  .job {
    margin-top: 1.25rem;
    opacity: 0;
    transform: translateY(8px);
    transition:
      opacity 0.5s var(--ease-out),
      transform 0.5s var(--ease-out);
  }
  .job.show {
    opacity: 1;
    transform: none;
  }
  .job.fading {
    opacity: 0;
    transform: none;
  }
  .jobhead {
    display: flex;
    align-items: center;
    gap: 0.85rem;
  }
  .file {
    flex: none;
    fill: none;
    stroke: var(--ink);
    stroke-width: 1.2;
  }
  .file rect {
    fill: #10b981;
    stroke: none;
  }
  .who {
    flex: 1;
    min-width: 0;
  }
  .name {
    font-weight: 600;
    font-size: 1rem;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    font: 400 0.75rem/1.5 var(--font-mono);
    color: var(--subtle);
  }
  .pct {
    font: 400 1.5rem/1 var(--font-mono);
    letter-spacing: -0.03em;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 1rem;
    margin: 1.25rem 0 0;
    padding: 1rem 0;
    border-block: 1px solid var(--line);
  }
  .stats dt {
    font-size: 0.75rem;
    color: var(--subtle);
  }
  .stats dd {
    margin: 0.2rem 0 0;
    font: 400 0.8125rem/1.1 var(--font-mono);
    color: var(--muted);
    white-space: nowrap;
  }
  .big {
    font-size: 1.75rem;
    letter-spacing: -0.04em;
    color: var(--ink);
  }
  .label {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    margin-bottom: 0.6rem;
    font-size: 0.75rem;
    color: var(--muted);
  }
  .note {
    color: var(--subtle);
  }
  .mapwrap {
    margin-top: 1.1rem;
  }
  .mapwrap canvas {
    width: 100%;
    height: 7rem;
  }
  .lower {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1.1fr);
    gap: 1.5rem;
    margin-top: 1.25rem;
  }
  .graph svg {
    width: 100%;
    height: 7.5rem;
    overflow: visible;
  }
  .graph .grid {
    stroke: var(--line);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
  .graph .area {
    fill: rgb(16 185 129 / 0.12);
  }
  .graph .line {
    fill: none;
    stroke: #047857;
    stroke-width: 1.75;
    vector-effect: non-scaling-stroke;
    stroke-linejoin: round;
  }
  .conns {
    list-style: none;
    padding: 0;
    display: grid;
    align-content: start;
    gap: 0.3rem;
    font-size: 0.75rem;
  }
  .conns li {
    display: grid;
    grid-template-columns: 8px 1.8rem minmax(0, 1fr) 4rem 2.4rem;
    align-items: center;
    gap: 0.55rem;
    height: 1.2rem;
  }
  .dot {
    width: 8px;
    height: 8px;
    background: linear-gradient(135deg, #6ee7b7, #10b981 55%, #0d9488);
    box-shadow: inset 0 1px 0 rgb(255 255 255 / 0.35);
  }
  .dot.off {
    background: #d4d4d8;
    box-shadow: none;
  }
  .cid,
  .rate {
    font-family: var(--font-mono);
    color: var(--subtle);
  }
  .rate {
    text-align: right;
    color: var(--ink);
  }
  .state {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
  }
  .rbar {
    height: 4px;
    background: var(--soft);
  }
  .rbar span {
    display: block;
    height: 100%;
    background: #10b981;
    transition: width 0.2s linear;
  }
  .done {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem 0.75rem;
    margin-top: 1.25rem;
    padding: 0.75rem 1rem;
    border-radius: 12px;
    background: #ecfdf5;
    color: #064e3b;
    font-size: 0.875rem;
  }
  .done:not(.show) {
    display: none;
  }
  .done.show {
    animation: appear 0.45s var(--ease-out);
  }
  .done svg {
    fill: none;
    stroke: #047857;
    stroke-width: 2;
  }
  .what {
    font-weight: 600;
  }
  .ok {
    font: 400 0.75rem/1 var(--font-mono);
    color: #047857;
  }
  .open {
    margin-left: auto;
    text-decoration: underline;
    text-underline-offset: 0.25em;
    text-decoration-color: rgb(4 120 87 / 0.4);
  }
  .main {
    display: flex;
    flex-direction: column;
  }
  .job:not(.show) {
    display: none;
  }
  .job.show:not(.fading) {
    animation: appear 0.55s var(--ease-out);
  }
  @keyframes appear {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
  }
  .recent {
    margin-top: 1.5rem;
    transition: margin 0.6s var(--ease-out);
  }
  .recent ul {
    list-style: none;
    padding: 0;
  }
  .recent li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 6rem 4rem;
    gap: 1rem;
    padding: 0.7rem 0;
    border-top: 1px solid var(--line);
    font-size: 0.8125rem;
  }
  .rname {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rsize {
    font-family: var(--font-mono);
    color: var(--subtle);
    text-align: right;
  }
  .rdone {
    color: #047857;
    text-align: right;
  }
  figcaption {
    margin-top: 1rem;
    text-align: center;
    font-size: 0.8125rem;
    color: var(--subtle);
  }

  @media (max-width: 52rem) {
    .body {
      grid-template-columns: minmax(0, 1fr);
      min-height: 0;
    }
    .side {
      display: none;
    }
    .main {
      padding: 1rem;
    }
    .stats {
      grid-template-columns: repeat(2, minmax(0, 1fr));
    }
    .lower {
      grid-template-columns: minmax(0, 1fr);
    }
    .app {
      display: none;
    }
    .note {
      display: none;
    }
  }
  @media (max-width: 30rem) {
    .controls svg {
      padding: 0 12px;
    }
    .open {
      margin-left: 0;
    }
  }
</style>
