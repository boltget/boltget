<!--
  The app icon drawn for the page (BRAND.md, "Small sizes"): Onyx glass, the
  small-size design, a solid white arrow on the dark, top-lit Onyx tile with a
  light rim. It is the brand's 24-unit reference drawing (GLASS and glassParts
  in brand/scripts/build.mjs, which snap it per size for the PNGs), kept as a
  vector here so it stays smooth at any display scale. The rim holds the dark
  tile on the dark theme too, so both themes show Onyx.

  Window, taskbar and tray icons stay the native per-size PNGs.
-->
<script lang="ts">
  interface Props {
    size?: number;
    label?: string;
  }

  let { size = 20, label }: Props = $props();
  const id = $props.id();

  // The solid arrow with soft corners, on the 24-unit artboard.
  const ARROW =
    "M10.4 4.6c0-.5.4-.9.9-.9h1.4c.5 0 .9.4.9.9v6.9h3.3c.8 0 1.2.9.6 1.5l-4.8 5c-.4.4-1 .4-1.4 0l-4.8-5c-.6-.6-.2-1.5.6-1.5h3.3z";
</script>

<svg
  class="app-mark"
  viewBox="0 0 24 24"
  width={size}
  height={size}
  role={label ? "img" : undefined}
  aria-label={label}
  aria-hidden={label ? undefined : "true"}
>
  <defs>
    <linearGradient id="{id}-tile" x1="0" y1="1" x2="0" y2="23" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#4a4d52" />
      <stop offset="0.45" stop-color="#1a1b1e" />
      <stop offset="1" stop-color="#050506" />
    </linearGradient>
    <radialGradient id="{id}-light" cx="12" cy="3" r="13" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#fff" stop-opacity="0.22" />
      <stop offset="1" stop-color="#fff" stop-opacity="0" />
    </radialGradient>
    <linearGradient id="{id}-rim" x1="0" y1="1" x2="0" y2="23" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#fff" stop-opacity="0.38" />
      <stop offset="0.35" stop-color="#fff" stop-opacity="0.06" />
      <stop offset="1" stop-color="#fff" stop-opacity="0.14" />
    </linearGradient>
    <linearGradient id="{id}-arrow" x1="0" y1="4" x2="0" y2="20" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#fff" />
      <stop offset="0.55" stop-color="#e9ebee" />
      <stop offset="1" stop-color="#a9adb4" />
    </linearGradient>
    <filter id="{id}-shadow" x="-30%" y="-30%" width="160%" height="160%">
      <feDropShadow dx="0" dy="0.9" stdDeviation="0.7" flood-color="#000" flood-opacity="0.55" />
    </filter>
  </defs>
  <rect class="tile" x="1" y="1" width="22" height="22" rx="5.6" fill="url(#{id}-tile)" />
  <rect class="light" x="1" y="1" width="22" height="22" rx="5.6" fill="url(#{id}-light)" />
  <rect
    class="rim"
    x="1.35"
    y="1.35"
    width="21.3"
    height="21.3"
    rx="5.25"
    fill="none"
    stroke="url(#{id}-rim)"
    stroke-width="0.7"
  />
  <path class="arrow" d={ARROW} fill="url(#{id}-arrow)" filter="url(#{id}-shadow)" />
</svg>

<style>
  .app-mark {
    display: block;
    flex: none;
  }

  @media (forced-colors: active) {
    .tile {
      fill: CanvasText;
    }
    .light,
    .rim {
      display: none;
    }
    .arrow {
      fill: Canvas;
      filter: none;
    }
  }
</style>
