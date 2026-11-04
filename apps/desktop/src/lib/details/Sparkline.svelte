<!--
  Throughput over the last minute (APP-DESIGN #10): one line and a faint
  area, scaled to the highest speed seen. Samples arrive at 2 Hz; the path is
  redrawn, never tweened. Until a minute has passed the time axis covers what
  there is (at least `minimum` samples), so a fresh download draws from the
  left edge instead of a sliver at the right; a dot marks the newest sample.
-->
<script lang="ts">
  interface Props {
    samples: readonly number[];
    capacity: number;
    /** The shortest time axis, in samples. */
    minimum?: number;
    label: string;
  }

  let { samples, capacity, minimum = 20, label }: Props = $props();

  const W = 320;
  const H = 56;

  const paths = $derived.by(() => {
    if (samples.length < 2) return null;
    const max = Math.max(...samples) * 1.15 || 1;
    const span = Math.min(capacity, Math.max(minimum, samples.length));
    const step = W / Math.max(1, span - 1);
    const offset = (span - samples.length) * step;
    const points = samples.map((v, i) => [offset + i * step, H - 2 - (v / max) * (H - 6)] as const);
    const line = points
      .map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)} ${y.toFixed(1)}`)
      .join("");
    const first = points[0] as readonly [number, number];
    const last = points[points.length - 1] as readonly [number, number];
    return {
      line,
      area: `${line}L${last[0].toFixed(1)} ${H}L${first[0].toFixed(1)} ${H}Z`,
      head: last,
    };
  });
</script>

<div class="wrap">
  <svg class="spark" viewBox="0 0 {W} {H}" preserveAspectRatio="none" role="img" aria-label={label}>
    <path class="grid" d="M0 {H / 2}H{W}" />
    {#if paths}
      <path class="area" d={paths.area} />
      <path class="line" d={paths.line} />
    {/if}
  </svg>
  {#if paths}
    <span
      class="head"
      style:left="{(paths.head[0] / W) * 100}%"
      style:top="{(paths.head[1] / H) * 100}%"
      aria-hidden="true"
    ></span>
  {/if}
</div>

<style>
  .wrap {
    position: relative;
  }
  /* Drawn in HTML so it stays round under the stretched viewBox. */
  .head {
    position: absolute;
    width: 5px;
    height: 5px;
    margin: -2.5px 0 0 -2.5px;
    border-radius: 50%;
    background: var(--color-accent);
  }
  .spark {
    width: 100%;
    height: 56px;
    overflow: visible;
  }
  .grid {
    stroke: var(--color-hairline);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
    stroke-dasharray: 2 3;
  }
  .area {
    fill: var(--color-accent-tint);
  }
  .line {
    fill: none;
    stroke: var(--color-accent);
    stroke-width: 1.5;
    stroke-linejoin: round;
    vector-effect: non-scaling-stroke;
  }
</style>
