<!--
  The Boltget mark, the geometry of brand/svg/mark.svg: solid ink on the
  light theme, the app icon's white-to-sky ramp on dark (BRAND.md, The mark). With `arrive`, the rows light up once, top to
  bottom, in the brand rhythm (BRAND.md, Motion: linear, 0.12 s stagger, the
  0.31 s rise) and stay lit; `paused` holds them dim until the moment it
  should play (when the window appears). With reduced motion it is static.

  For 24 px and up. Smaller sizes use the pixel-snapped icons in brand/svg.
  At 108 px (or 54 px) every square lands on whole pixels at 100% scale.
-->
<script lang="ts">
  interface Props {
    size?: number;
    arrive?: boolean;
    paused?: boolean;
    label?: string;
  }

  let { size = 108, arrive = false, paused = false, label }: Props = $props();
  const id = $props.id();

  // Rows of the arrow, top to bottom: [first column, last column] on the
  // 12-unit grid, each square 10 units with a 2-unit gap.
  const ROWS: [number, number][] = [
    [3, 5],
    [3, 5],
    [3, 5],
    [3, 5],
    [0, 8],
    [1, 7],
    [2, 6],
    [3, 5],
    [4, 4],
  ];

  const rows = ROWS.map(([first, last], row) => {
    const y = 37 + row * 12;
    let d = "";
    for (let col = first; col <= last; col += 1) d += `M${37 + col * 12} ${y}h10v10h-10z`;
    return d;
  });
</script>

<svg
  class="mark"
  class:arrive
  class:paused
  viewBox="36 36 108 108"
  width={size}
  height={size}
  role={label ? "img" : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
>
  <defs>
    <linearGradient id="{id}-g" gradientUnits="userSpaceOnUse" x1="0" y1="36" x2="0" y2="144">
      <stop class="s0" offset="0" />
      <stop class="s1" offset="0.42" />
      <stop class="s2" offset="0.7" />
      <stop class="s3" offset="1" />
    </linearGradient>
  </defs>
  {#each rows as d, row (row)}
    <path {d} fill="url(#{id}-g)" style:--row={row} />
  {/each}
</svg>

<style>
  .mark {
    flex: none;
  }
  .s0 {
    stop-color: var(--color-mark-0);
  }
  .s1 {
    stop-color: var(--color-mark-1);
  }
  .s2 {
    stop-color: var(--color-mark-2);
  }
  .s3 {
    stop-color: var(--color-mark-3);
  }
  .arrive path {
    opacity: var(--motion-arrive-rest);
    animation: arrive var(--motion-arrive-rise) linear both;
    animation-delay: calc(var(--row) * var(--motion-arrive-stagger));
  }
  .paused path {
    animation-play-state: paused;
  }
  @keyframes arrive {
    from {
      opacity: var(--motion-arrive-rest);
    }
    to {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .arrive path {
      animation: none;
      opacity: 1;
    }
  }
</style>
