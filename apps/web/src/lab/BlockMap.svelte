<!--
  Lab B centerpiece: a simulated run of the v0.1 command-line tool, in its real
  output format (crates/cli/src/display.rs), and behind it the whole hero as a
  map of the file being downloaded. Each square of the page is a slice of the
  file; each bright edge is a connection writing. When a connection finishes
  early it takes over half of the largest part left, and a new edge appears in
  the middle of it. When the file is complete the page holds, drains, and the
  next run starts.

  The canvas is moved into the closest [data-map-host] (the hero section) and fills it. Server
  render and reduced motion show the finished run as a still. The loop pauses
  off screen and in hidden tabs.
-->
<script lang="ts" module>
  import { DownloadSim, humanBytes, humanDuration, type SimOptions } from "./sim";

  export const SIM: SimOptions = {
    total: 420,
    start: 4,
    max: 16,
    rate: [9, 13],
    cap: 58,
    every: 0.6,
    minSplit: 4,
    seed: 7,
  };

  const SPIN = ["⠁", "⠂", "⠄", "⡀", "⢀", "⠠", "⠐", "⠈"];
  const PARTIAL = ["▏", "▎", "▍", "▌", "▋", "▊", "▉"];

  function finished(): DownloadSim {
    const s = new DownloadSim(SIM);
    while (!s.finished) s.step();
    return s;
  }
  const FINAL = finished();

  /** indicatif's `{bar:N}` with progress_chars("█▉▊▋▌▍▎▏ "). */
  function bar(f: number, width: number): [string, string] {
    const exact = Math.max(0, Math.min(1, f)) * width;
    const full = Math.floor(exact);
    let out = "█".repeat(full);
    if (full < width) {
      const rem = exact - full;
      out += rem > 0.05 ? PARTIAL[Math.min(6, Math.floor(rem * 7))] : " ";
    }
    return [out, " ".repeat(Math.max(0, width - out.length))];
  }

  /** The CLI's one-line segment map (display.rs, segment_map). */
  function segmentMap(sim: DownloadSim, width: number): string {
    const cell = SIM.total / width;
    let out = "";
    for (let i = 0; i < width; i++) {
      const f = sim.cell(i * cell, (i + 1) * cell).f;
      out += f >= 1 ? "█" : f >= 0.66 ? "▓" : f >= 0.33 ? "▒" : f > 0 ? "░" : "·";
    }
    return `[${out}] ${sim.segs.length} segments`;
  }
</script>

<script lang="ts">
  import { onMount } from "svelte";

  interface Strings {
    prompt: string;
    title: string;
    url: string;
    name: string;
    folder: string;
    hash: string;
    demoLabel: string;
    demoNote: string;
    legend: string;
    honestyLink: string;
  }

  let { s }: { s: Strings } = $props();

  interface Line {
    text: string;
    cls: string;
  }

  let canvas: HTMLCanvasElement;
  let term: HTMLElement;
  let pre: HTMLElement;
  /** Characters per terminal line, from the terminal's width. */
  let cpl = 120;

  // The command line: typed parts, and parts pasted at once (URL, digest).
  const parts = $derived([
    { text: "boltget get ", paste: false },
    { text: s.url, paste: true },
    { text: " --checksum ", paste: false },
    { text: `sha256:${s.hash}`, paste: true },
  ]);
  const KEY = 0.045;
  const PASTE = 0.28;
  const T0 = 0.6;
  const typeEnd = $derived(parts.reduce((t, p) => t + (p.paste ? PASTE : p.text.length * KEY), T0));
  const ENTER = $derived(typeEnd + 0.4);
  const PROBE = $derived(ENTER + 0.6);

  function typedAt(clock: number): string {
    let t = T0;
    let out = "";
    for (const p of parts) {
      if (clock < t) return out;
      if (p.paste) {
        t += PASTE;
        if (clock < t) return out;
        out += p.text;
      } else {
        const n = Math.min(p.text.length, Math.floor((clock - t) / KEY));
        out += p.text.slice(0, n);
        t += p.text.length * KEY;
        if (n < p.text.length) return out;
      }
    }
    return out;
  }

  function linesAt(clock: number, sim: DownloadSim, spin: number): { lines: Line[]; caret: boolean } {
    const cmd = typedAt(clock);
    const lines: Line[] = [{ text: `${s.prompt} ${cmd}`, cls: "cmd" }];
    if (clock < ENTER) return { lines, caret: true };
    const sp = SPIN[spin % SPIN.length]!;
    if (clock < PROBE) {
      lines.push({ text: `${sp} connecting`, cls: "spin" });
      return { lines, caret: false };
    }
    if (!sim.finished) {
      lines.push({ text: `  ${s.name} -> ${s.folder}`, cls: "out" });
      const w = Math.max(6, Math.min(32, cpl - 71));
      const f = sim.downloaded / SIM.total;
      const [filled, empty] = bar(f, w);
      const eta = sim.eta();
      const pct = String(Math.floor(f * 100)).padStart(3);
      const prefix = `${humanBytes(sim.speed).padStart(10)}/s  ETA ${(eta === null ? "-" : humanDuration(eta)).padEnd(11)} ${String(sim.active).padStart(2)} conn`;
      lines.push({
        text: `\u0000${sp}\u0001 \u0002${filled}\u0003${empty} ${humanBytes(sim.downloaded).padStart(10)} / ${humanBytes(SIM.total).padEnd(10)} ${pct}%  ${prefix}`,
        cls: "bar",
      });
      lines.push({ text: `  ${segmentMap(sim, Math.max(12, Math.min(48, cpl - 20)))}`, cls: "dim" });
      return { lines, caret: false };
    }
    const secs = sim.finishedAt;
    lines.push({ text: `  ${s.name} -> ${s.folder}`, cls: "out" });
    lines.push({
      text: `Saved ${s.folder}\\${s.name} (${humanBytes(SIM.total)}) in ${secs.toFixed(1)}s, ${humanBytes(SIM.total / secs)}/s average, ${sim.peakConnections} connections, ${sim.segs.length} segments`,
      cls: "out",
    });
    lines.push({ text: `Checksum OK (sha256:${s.hash})`, cls: "ok" });
    lines.push({ text: `${s.prompt} `, cls: "cmd" });
    return { lines, caret: true };
  }

  let term$ = $state.raw(linesAt(999, FINAL, 0));

  // Splits a bar line into its coloured pieces (spinner, filled bar).
  function pieces(text: string): { t: string; c: string }[] {
    const m = /^\u0000(.*?)\u0001 \u0002(.*?)\u0003(.*)$/su.exec(text);
    if (!m) return [{ t: text, c: "" }];
    return [
      { t: m[1] ?? "", c: "spin" },
      { t: " ", c: "" },
      { t: m[2] ?? "", c: "fill" },
      { t: m[3] ?? "", c: "" },
    ];
  }

  // ---- The page as a file map ---------------------------------------------------
  const STOPS = [
    [167, 243, 208],
    [52, 211, 153],
    [5, 150, 105],
  ] as const;
  function gradient(d: number): string {
    const k = Math.max(0, Math.min(1, d)) * 2;
    const [a, b] = k < 1 ? [STOPS[0], STOPS[1]] : [STOPS[1], STOPS[2]];
    const u = k < 1 ? k : k - 1;
    return `${Math.round(a[0] + (b[0] - a[0]) * u)},${Math.round(a[1] + (b[1] - a[1]) * u)},${Math.round(a[2] + (b[2] - a[2]) * u)}`;
  }

  let arrived = new Float32Array(0);
  let colors: string[] = [];
  let layout = { w: 0, h: 0, pitch: 16, size: 13, cols: 0, rows: 0 };

  function measure() {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = canvas.clientWidth;
    const h = canvas.clientHeight;
    const pitch = w < 640 ? 12 : 16;
    const size = pitch - (w < 640 ? 2 : 3);
    const cols = Math.ceil(w / pitch);
    const rows = Math.ceil(h / pitch);
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    canvas.getContext("2d")?.setTransform(dpr, 0, 0, dpr, 0, 0);
    if (cols * rows !== arrived.length) arrived = new Float32Array(cols * rows).fill(-1);
    colors = Array.from({ length: cols * rows }, (_, i) =>
      gradient(((i % cols) / cols + Math.floor(i / cols) / rows) / 2),
    );
    layout = { w, h, pitch, size, cols, rows };
    if (pre) {
      const cs = getComputedStyle(pre);
      const inner = pre.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
      cpl = Math.floor(inner / (parseFloat(cs.fontSize) * 0.6));
    }
  }

  /** drain: 0 = live, 1 = fully back to rest. */
  function drawField(sim: DownloadSim, drain: number) {
    const ctx = canvas?.getContext("2d");
    if (!ctx) return;
    const { w, h, pitch, size, cols, rows } = layout;
    const n = cols * rows;
    const slice = SIM.total / n;
    ctx.clearRect(0, 0, w, h);
    const off = (pitch - size) / 2;
    for (let i = 0; i < n; i++) {
      const x = (i % cols) * pitch + off;
      const y = Math.floor(i / cols) * pitch + off;
      const c = sim.cell(i * slice, (i + 1) * slice);
      let a = 0;
      if (c.f >= 1) {
        if (arrived[i]! < 0) arrived[i] = sim.t;
        a = 0.14 + 0.72 * Math.exp(-(sim.t - arrived[i]!) / 0.6);
      } else if (c.f > 0) {
        a = 0.14 * c.f;
      }
      if (c.head && !sim.finished) a = 0.95;
      a *= 1 - drain;
      if (a < 0.035) {
        ctx.fillStyle = "rgba(255,255,255,0.035)";
      } else {
        ctx.fillStyle = `rgba(${colors[i]},${a.toFixed(3)})`;
      }
      ctx.fillRect(x, y, size, size);
    }
  }

  onMount(() => {
    let current: DownloadSim = FINAL;
    // The map fills the hero, not the island: move the canvas to its host.
    canvas.closest("[data-map-host]")?.prepend(canvas);
    measure();
    drawField(FINAL, 0);
    const ro = new ResizeObserver(() => {
      measure();
      drawField(current, 0);
    });
    ro.observe(canvas);

    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
    if (reduced.matches) return () => ro.disconnect();

    let sim = new DownloadSim(SIM);
    current = sim;
    arrived.fill(-1);
    let clock = 0;
    let doneAt = 0;
    let raf = 0;
    let last = 0;
    let lastText = 0;
    let visible = true;

    const update = (now: number, force = false) => {
      let drain = 0;
      if (clock >= PROBE && !sim.finished) {
        const target = clock - PROBE;
        while (!sim.finished && sim.t < target) sim.step();
        if (sim.finished) doneAt = clock;
      }
      if (sim.finished) {
        const since = clock - doneAt;
        if (since > 4) drain = Math.min(1, (since - 4) / 0.9);
        if (since > 5.2) {
          sim = new DownloadSim(SIM);
          current = sim;
          arrived.fill(-1);
          clock = 0;
          doneAt = 0;
          drain = 0;
        }
      }
      if (force || now - lastText > 50) {
        lastText = now;
        term$ = linesAt(clock, sim, Math.floor(clock / 0.12));
      }
      drawField(sim, drain);
    };

    const at = Number(new URLSearchParams(location.search).get("at"));
    if (at > 0) {
      // Replay up to that moment so each square knows when it arrived.
      while (clock < at) {
        clock = Math.min(at, clock + 1 / 30);
        update(0, clock >= at);
      }
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
    io.observe(term);
    document.addEventListener("visibilitychange", schedule);
    term$ = linesAt(0, sim, 0);
    schedule();
    return () => {
      cancelAnimationFrame(raf);
      io.disconnect();
      ro.disconnect();
      document.removeEventListener("visibilitychange", schedule);
    };
  });
</script>

<canvas class="field" bind:this={canvas} aria-hidden="true"></canvas>

<figure class="term" bind:this={term} aria-label={s.demoLabel}>
  <div class="chrome" aria-hidden="true">
    <span class="tab">{s.title}</span>
    <span class="controls">
      <svg viewBox="0 0 10 10" width="10" height="10"><path d="M0 5.5h10" /></svg>
      <svg viewBox="0 0 10 10" width="10" height="10"><path d="M.5.5h9v9h-9z" /></svg>
      <svg viewBox="0 0 10 10" width="10" height="10"><path d="M0 0l10 10M10 0L0 10" /></svg>
    </span>
  </div>
  <pre bind:this={pre} aria-hidden="true">{#each term$.lines as line, i (i)}<span class={`l ${line.cls}`}
        >{#if line.cls === "bar"}{#each pieces(line.text) as p, j (j)}<span class={p.c}>{p.t}</span>{/each}{:else}{line.text}{/if}{#if term$.caret && i === term$.lines.length - 1}<span
            class="caret"
          ></span>{/if}</span
      >
{/each}</pre>
  <figcaption>
    <span>{s.legend}</span>
    <span>{s.demoNote} <a class="tlink" href="/benchmarks/">{s.honestyLink}</a></span>
  </figcaption>
</figure>

<style>
  .field {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    z-index: 0;
    pointer-events: none;
  }
  .term {
    position: relative;
    z-index: 2;
    margin: 0;
  }
  .chrome {
    display: flex;
    align-items: center;
    height: 38px;
    padding-left: 0.9rem;
    border: 1px solid #27272a;
    border-bottom: 0;
    border-radius: 12px 12px 0 0;
    background: #141418;
    font-size: 0.75rem;
    color: #a1a1aa;
  }
  .tab {
    padding: 0.3rem 0.7rem;
    border-radius: 6px;
    background: #1c1c21;
    color: #e4e4e7;
  }
  .controls {
    display: flex;
    height: 100%;
    margin-left: auto;
  }
  .controls svg {
    box-sizing: content-box;
    padding: 0 16px;
    height: 100%;
    fill: none;
    stroke: #71717a;
  }
  pre {
    margin: 0;
    height: 15.5rem;
    padding: 1rem 1.1rem;
    overflow: hidden;
    border: 1px solid #27272a;
    border-radius: 0 0 12px 12px;
    background: #0c0c10;
    color: #d4d4d8;
    font: 400 0.8125rem/1.65 "Geist Mono", "Cascadia Mono", Consolas, ui-monospace, monospace;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-variant-ligatures: none;
    font-feature-settings: "calt" 0, "liga" 0;
  }
  .cmd {
    color: #fafafa;
  }
  .spin,
  .fill {
    color: #6ee7b7;
  }
  .dim {
    color: #71717a;
  }
  .ok {
    color: #34d399;
  }
  .caret {
    display: inline-block;
    width: 0.6em;
    height: 1.15em;
    vertical-align: -0.22em;
    background: #d4d4d8;
    animation: blink 1s steps(1) infinite;
  }
  @keyframes blink {
    50% {
      opacity: 0;
    }
  }
  figcaption {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: 0.4rem 1.5rem;
    margin-top: 0.9rem;
    font-size: 0.8125rem;
    color: #a1a1aa;
  }
  figcaption a {
    color: #fafafa;
  }
  @media (max-width: 40rem) {
    pre {
      height: 19rem;
      font-size: 0.6875rem;
      padding: 0.85rem;
    }
  }
</style>
