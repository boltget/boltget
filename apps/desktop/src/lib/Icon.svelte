<!--
  16 px line icons for the shell, drawn on a 16-unit grid with square caps
  and mitred joins (the brand is made of square pixels). Decorative only:
  the control that holds an icon carries the accessible name.
-->
<script lang="ts" module>
  const PATHS = {
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
  } as const;

  export type IconName = keyof typeof PATHS;
</script>

<script lang="ts">
  interface Props {
    name: IconName;
    size?: number;
  }

  let { name, size = 16 }: Props = $props();
</script>

<svg class="icon" viewBox="0 0 16 16" width={size} height={size} aria-hidden="true">
  <path d={PATHS[name]} />
</svg>

<style>
  .icon {
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.25;
    stroke-linecap: square;
    stroke-linejoin: miter;
  }
</style>
