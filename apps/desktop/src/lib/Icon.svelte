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
    file: "M3.5 2h5.5l3.5 3.5v8.5h-9ZM9 2v3.5h3.5",
    pause: "M5.5 3.5v9M10.5 3.5v9",
    play: "M5 3v10l8-5Z",
    retry: "M13 8a5 5 0 1 1-1.46-3.54M12 1.75v3h-3",
    folder: "M2 4h4.5l1.5 1.5h6v7.5H2Z",
    open: "M9 2.5h4.5V7M13.5 2.5 7.5 8.5M11.5 9.5v4h-9v-9h4",
    link: "M6.5 9.5l3-3M7 4.5l1.2-1.2a2.6 2.6 0 0 1 3.7 3.7L10.7 8.2M9 11.5l-1.2 1.2a2.6 2.6 0 0 1-3.7-3.7L5.3 7.8",
    trash: "M2.5 4.5h11M6 4.5v-2h4v2M4 4.5l.75 9h6.5l.75-9",
    close: "M4 4l8 8M12 4l-8 8",
    more: "M3 7.5h1v1H3ZM7.5 7.5h1v1h-1ZM12 7.5h1v1h-1Z",
    check: "M3.5 8.5l3 3 6-6.5",
    alert: "M8 2.5l6 10.5H2ZM8 6.5v3M8 11v.5",
    lock: "M4 7.5h8v6H4ZM5.5 7.5v-2a2.5 2.5 0 0 1 5 0v2",
    sort: "M5 3v10M2.5 10.5 5 13l2.5-2.5M11 13V3M8.5 5.5 11 3l2.5 2.5",
    pane: "M2.5 3.5h11v9h-11ZM9.5 3.5v9",
    copy: "M5.5 5.5h8v8h-8ZM10.5 5.5v-3h-8v8h3",
    chevron: "M6 3.5 10.5 8 6 12.5",
    down: "M3.5 6 8 10.5 12.5 6",
    keyboard: "M1.5 4.5h13v7h-13ZM4 7h1M7.5 7h1M11 7h1M5 9.5h6",
    info: "M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11ZM8 7v4M8 5v.5",
    back: "M7 3.5 2.5 8 7 12.5M2.5 8h11",
    globe:
      "M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11ZM2.5 8h11M8 2.5c-1.6 1.5-2.4 3.3-2.4 5.5s.8 4 2.4 5.5M8 2.5c1.6 1.5 2.4 3.3 2.4 5.5s-.8 4-2.4 5.5",
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
