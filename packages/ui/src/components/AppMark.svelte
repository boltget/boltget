<!--
  The app icon drawn for the page (BRAND.md, The app icon): the Onyx tile with
  a white arrow on the light theme, its white twin (a paper tile with a
  hairline edge and an ink arrow) on dark. The arrow is one stepped
  silhouette, not 37 separate pixels, so it stays crisp at any size and any
  display scale; the pixel-gap icons in brand/svg only survive at the native
  size they were drawn for. The theme switch is light-dark(), which follows
  color-scheme (base.css).

  Window, taskbar and tray icons stay the native per-size PNGs.
-->
<script lang="ts">
  interface Props {
    size?: number;
    label?: string;
  }

  let { size = 20, label }: Props = $props();
  const id = $props.id();

  // The 9x9 arrow of the mark, cell for cell, as one outline.
  const ARROW = "M3 0H6V4H9V5H8V6H7V7H6V8H5V9H4V8H3V7H2V6H1V5H0V4H3Z";
</script>

<svg
  class="app-mark"
  viewBox="0 0 18 18"
  width={size}
  height={size}
  role={label ? "img" : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
>
  <defs>
    <linearGradient id="{id}-tile" x1="0" y1="0" x2="0.6" y2="1">
      <stop class="t0" offset="0" />
      <stop class="t1" offset="1" />
    </linearGradient>
  </defs>
  <rect class="tile" x="0.25" y="0.25" width="17.5" height="17.5" rx="4.4" fill="url(#{id}-tile)" />
  <path class="arrow" d={ARROW} transform="translate(3.5 3.5) scale(1.2222)" />
</svg>

<style>
  .app-mark {
    display: block;
    flex: none;
  }
  .t0 {
    stop-color: light-dark(var(--graphite-800), var(--white));
  }
  .t1 {
    stop-color: light-dark(var(--ink), var(--graphite-100));
  }
  /* The white twin's hairline keeps its edge on the dark chrome; the Onyx
     tile needs none. */
  .tile {
    stroke: light-dark(transparent, var(--graphite-300));
    stroke-width: 0.5;
  }
  .arrow {
    fill: light-dark(var(--white), var(--ink));
  }

  @media (forced-colors: active) {
    .tile {
      fill: CanvasText;
      stroke: none;
    }
    .arrow {
      fill: Canvas;
    }
  }
</style>
