<!--
  The live file map (APP-DESIGN #11, BRAND.md "Pixels on the web"): the file
  as a field of square chunks, each a window onto one gradient across the
  whole field. A square eases in as its bytes land and carries a brief light;
  the square a connection is writing is deep ink with a soft halo. When the
  download finishes the field settles into one calm tint.

  Drawn on a canvas, only while something changes and at 30 fps or less.
  Reduced motion shows every square in its settled state at once. Hovering a
  square shows its byte range and the connection that owns it.
-->
<script lang="ts">
  import { t } from "@boltget/ui/messages";
  import { hexToRgba, themeColor, type ThemeColorName } from "@boltget/ui/theme";
  import { onMount } from "svelte";
  import type { SegmentSpan } from "../bindings";
  import { formatBytes } from "../format";
  import { currentTheme } from "../theme";
  import {
    cellAt,
    cellRange,
    contiguous,
    coverage,
    grid as makeGrid,
    rampPosition,
    type Coverage,
  } from "./fileMap";

  interface Props {
    segments: SegmentSpan[] | null;
    total: number;
    downloaded: number;
    complete: boolean;
    live: boolean;
    label: string;
    rows?: number;
  }

  let { segments, total, downloaded, complete, live, label, rows = 8 }: Props = $props();

  let canvas: HTMLCanvasElement | undefined = $state();
  let width = $state(0);
  let tip = $state<{ x: number; y: number; text: string } | null>(null);

  const g = $derived(makeGrid(width, rows));
  const cells = $derived(g.cols * g.rows);
  const data = $derived.by((): Coverage => {
    if (complete) return contiguous(1, cells);
    if (segments && segments.length) return coverage(segments, total, cells);
    return contiguous(total > 0 ? downloaded / total : 0, cells);
  });

  const reduced =
    typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

  // When each cell became complete (seconds, performance clock); NaN: not yet.
  let arrived = new Float64Array(0);
  let settleStart = NaN;
  let firstDraw = true;
  let frame = 0;
  let lastFrame = 0;

  type Rgb = [number, number, number];
  let colors: Record<string, Rgb> & { emptyAlpha?: number } = {};

  function rgb(name: ThemeColorName): Rgb {
    const [r, g2, b] = hexToRgba(themeColor(currentTheme(), name));
    return [r, g2, b];
  }

  function readColors() {
    const theme = currentTheme();
    const empty = hexToRgba(themeColor(theme, "map-empty"));
    colors = {
      empty: [empty[0], empty[1], empty[2]],
      m0: rgb("map-0"),
      m1: rgb("map-1"),
      m2: rgb("map-2"),
      m3: rgb("map-3"),
      head: rgb("map-head"),
      light: rgb("map-light"),
      depth: rgb("map-depth"),
    };
    colors.emptyAlpha = empty[3] / 255;
  }

  function ramp(u: number): Rgb {
    const stops = [colors.m0, colors.m1, colors.m2, colors.m3] as Rgb[];
    const x = Math.max(0, Math.min(1, u)) * (stops.length - 1);
    const i = Math.min(stops.length - 2, Math.floor(x));
    const k = x - i;
    const a = stops[i] as Rgb;
    const b = stops[i + 1] as Rgb;
    return [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k];
  }

  function hash(i: number): number {
    let h = Math.imul(i ^ 0x9e3779b9, 0x85ebca6b);
    h ^= h >>> 13;
    h = Math.imul(h, 0xc2b2ae35);
    return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
  }

  const easeOut = (x: number) => 1 - Math.pow(1 - Math.max(0, Math.min(1, x)), 3);
  const css = (c: Rgb) => `rgb(${c[0] | 0},${c[1] | 0},${c[2] | 0})`;

  /** Draws one frame; returns true while something is still easing. */
  function draw(): boolean {
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx || g.width === 0) return false;
    if (!colors.m0) readColors();
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = Math.round(g.width * dpr);
    const h = Math.round(g.height * dpr);
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
    }
    if (arrived.length !== cells) {
      arrived = new Float64Array(cells).fill(NaN);
      firstDraw = true;
    }
    const now = performance.now() / 1000;
    let busy = false;

    // Settling: the field calms over 0.7 s once complete.
    if (complete && Number.isNaN(settleStart)) settleStart = firstDraw || reduced ? -1e9 : now;
    if (!complete) settleStart = NaN;
    const settle = complete ? easeOut((now - settleStart) / 0.7) : 0;
    if (complete && settle < 1) busy = true;

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, g.width, g.height);
    const { cell, gap, cols } = g;
    const band = Math.max(1, Math.round(cell * 0.38));
    const depth = Math.max(1, Math.round(cell * 0.1));

    for (let i = 0; i < cells; i += 1) {
      const col = i % cols;
      const row = Math.floor(i / cols);
      const x = g.left + col * (cell + gap);
      const y = row * (cell + gap);
      const f = data.fill[i] ?? 0;
      const variance = hash(i);

      // The ground: pale and slightly uneven, so the field reads as material.
      ctx.globalAlpha = (colors.emptyAlpha ?? 0.07) * (0.7 + 0.6 * variance);
      ctx.fillStyle = css(colors.empty as Rgb);
      ctx.fillRect(x, y, cell, cell);

      if (f <= 0) {
        arrived[i] = NaN;
        continue;
      }
      let [r, gg, b] = ramp(rampPosition(col, row, cols, g.rows));
      let alpha: number;
      if (f >= 1) {
        if (Number.isNaN(arrived[i])) arrived[i] = firstDraw || reduced ? -1e9 : now;
        const age = now - (arrived[i] ?? 0);
        if (age < 0.9) busy = true;
        alpha = 0.35 + 0.65 * easeOut(age / 0.45);
        const flash = Math.exp(-age / 0.32) * 0.55;
        const light = colors.light as Rgb;
        r += (light[0] - r) * flash;
        gg += (light[1] - gg) * flash;
        b += (light[2] - b) * flash;
        // Settled: one calm field, a touch uneven.
        alpha = alpha + (0.42 + 0.1 * variance - alpha) * settle;
      } else {
        arrived[i] = NaN;
        alpha = 0.16 + 0.34 * f;
      }
      ctx.globalAlpha = alpha;
      ctx.fillStyle = css([r, gg, b]);
      ctx.fillRect(x, y, cell, cell);
      if (f >= 1) {
        // Inner light at the top, a hairline of depth at the bottom.
        ctx.globalAlpha = alpha * 0.16;
        ctx.fillStyle = css(colors.light as Rgb);
        ctx.fillRect(x, y, cell, band);
        ctx.globalAlpha = alpha * 0.16;
        ctx.fillStyle = css(colors.depth as Rgb);
        ctx.fillRect(x, y + cell - depth, cell, depth);
      }
      if (live && !complete && (data.heads[i] ?? 0) > 0) {
        ctx.globalAlpha = 0.24;
        ctx.fillStyle = css(colors.m1 as Rgb);
        ctx.fillRect(x - gap / 2, y - gap / 2, cell + gap, cell + gap);
        ctx.globalAlpha = 1;
        ctx.fillStyle = css(colors.head as Rgb);
        ctx.fillRect(x, y, cell, cell);
      }
    }
    ctx.globalAlpha = 1;
    firstDraw = false;
    return busy;
  }

  // At most 30 frames a second, and only while something is easing.
  function schedule() {
    if (frame) return;
    frame = requestAnimationFrame(function loop(time) {
      frame = 0;
      if (time - lastFrame < 33) {
        frame = requestAnimationFrame(loop);
        return;
      }
      lastFrame = time;
      if (draw()) schedule();
    });
  }

  $effect(() => {
    // Redraw when the data, the size or the state changes.
    void data;
    void g;
    void live;
    void complete;
    schedule();
  });

  onMount(() => {
    const redraw = () => {
      readColors();
      schedule();
    };
    const observer = new MutationObserver(redraw);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-theme"],
    });
    const query = matchMedia("(prefers-color-scheme: dark)");
    query.addEventListener("change", redraw);
    return () => {
      observer.disconnect();
      query.removeEventListener("change", redraw);
      cancelAnimationFrame(frame);
    };
  });

  function onpointermove(event: PointerEvent) {
    if (!canvas || total <= 0) return;
    const box = canvas.getBoundingClientRect();
    const x = event.clientX - box.left;
    const y = event.clientY - box.top;
    const index = cellAt(g, x, y);
    if (index < 0) {
      tip = null;
      return;
    }
    const [from, to] = cellRange(index, cells, total);
    const range = t("details.map.tip", { from: formatBytes(from), to: formatBytes(to) });
    const owner = data.owners[index] ?? 0;
    const center = g.left + (index % g.cols) * (g.cell + g.gap) + g.cell / 2;
    tip = {
      // Kept inside the pane: the tip is centred on the square where it fits.
      x: Math.max(90, Math.min(g.width - 90, center)),
      y: Math.floor(index / g.cols) * (g.cell + g.gap),
      text:
        owner > 0 && !complete
          ? t("details.map.tip_connection", { range, connection: owner })
          : range,
    };
  }
</script>

<div class="map" bind:clientWidth={width} role="img" aria-label={label}>
  <canvas
    bind:this={canvas}
    style:height="{g.height}px"
    aria-hidden="true"
    {onpointermove}
    onpointerleave={() => (tip = null)}
  ></canvas>
  {#if tip}
    <span class="tip" style:transform="translate({tip.x}px, {tip.y}px)">{tip.text}</span>
  {/if}
</div>

<style>
  .map {
    position: relative;
  }
  canvas {
    display: block;
    width: 100%;
  }
  .tip {
    position: absolute;
    left: 0;
    top: 0;
    z-index: 2;
    margin-top: -6px;
    padding: 3px 8px;
    border-radius: var(--radius-control);
    background: var(--color-tooltip);
    color: var(--color-on-tooltip);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    white-space: nowrap;
    pointer-events: none;
    translate: -50% -100%;
  }
</style>
