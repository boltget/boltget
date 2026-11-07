<!--
  One download (APP-DESIGN #3). Comfortable: two lines in 40 px, the name
  with the state or primary action at the right, then a thin progress bar and
  one muted status line. Compact: one line in 32 px with the bar as a 2 px
  underline. Digits repaint at 2 Hz and the time left moves only when it
  changes by more than 10% (#27); the fill glides across each tick with a
  transform (never width).

  Selection and keyboard live on the list (one listbox with
  aria-activedescendant); the row's buttons are a pointer shortcut for what
  the keyboard and the context menu also reach.
-->
<script lang="ts">
  import IconButton from "@boltget/ui/components/IconButton.svelte";
  import { t, type PlainMessageKey } from "@boltget/ui/messages";
  import { untrack } from "svelte";
  import { app } from "../app.svelte";
  import { clock } from "../clock.svelte";
  import { errorText, type Download } from "../engine";
  import {
    displayName,
    formatBytes,
    formatEta,
    formatProgress,
    formatSpeed,
    formatUpcoming,
    formatWhen,
    hostOf,
    percent,
    secondsLeft,
    stableEta,
  } from "../format";
  import Icon, { type IconName } from "../Icon.svelte";
  import { primaryAction, type ActionId } from "./actions";
  import { categoryIcon } from "./category";
  import { isRunning, needsAttention } from "./model";

  interface Props {
    download: Download;
    selected: boolean;
    active: boolean;
    /** The pointer is over the row (tracked by the list, one listener). */
    hovered: boolean;
    /** The keyboard's row while the list has keyboard focus: draws the ring. */
    ring: boolean;
    compact: boolean;
    position: number;
    count: number;
    onmore: (event: MouseEvent) => void;
    /** It can be dragged (to its queue's order, or onto a queue). */
    draggable?: boolean;
    /** It is being dragged. */
    dragged?: boolean;
  }

  let {
    download: d,
    selected,
    active,
    hovered,
    ring,
    compact,
    position,
    count,
    onmore,
    draggable = false,
    dragged = false,
  }: Props = $props();

  const ACTION_ICONS: Record<ActionId, IconName> = {
    pause: "pause",
    resume: "play",
    retry: "retry",
    restart: "retry",
    refresh: "link",
    open: "open",
    show: "folder",
    copy: "link",
    remove: "trash",
    delete: "trash",
  };

  const name = $derived(displayName(d.fileName, d.url));
  const icon = $derived(categoryIcon(name));
  const running = $derived(isRunning(d.state));
  const attention = $derived(needsAttention(d.state));
  const action = $derived(primaryAction(d.state));
  const pct = $derived(percent(d.downloaded, d.total));
  const fraction = $derived(
    d.state === "completed" ? 1 : d.total ? Math.min(1, d.downloaded / d.total) : 0,
  );

  // Digits sampled on the shared 2 Hz beat.
  let bps = $state(0);
  let eta = $state<number | null>(null);
  $effect(() => {
    void clock.beat;
    untrack(() => {
      bps = d.bps;
      eta = running ? stableEta(eta, secondsLeft(d.downloaded, d.total, d.bps)) : null;
    });
  });
  // The first frame shows real numbers, not zeros.
  $effect.pre(() => {
    untrack(() => {
      if (bps === 0) bps = d.bps;
    });
  });

  const join = (first: string, second: string) => t("status.join", { first, second });

  const status = $derived.by(() => {
    switch (d.state) {
      case "downloading": {
        let text = formatProgress(d.downloaded, d.total);
        if (bps > 0) text = join(text, formatSpeed(bps));
        if (eta !== null) text = join(text, formatEta(eta));
        return text;
      }
      case "retrying":
        return join(formatProgress(d.downloaded, d.total), t("state.retrying"));
      case "verifying":
        return join(d.total ? formatBytes(d.total) : "", t("state.verifying"));
      case "queued":
        return d.startAt !== null
          ? join(t("row.starts", { time: formatUpcoming(d.startAt, clock.now) }), hostOf(d.url))
          : join(t("row.waiting_in", { queue: app.queueName(d.queue) }), hostOf(d.url));
      case "probing":
        return hostOf(d.url);
      case "completed":
        return join(formatBytes(d.total ?? d.downloaded), hostOf(d.url));
      case "paused":
      case "cancelled":
        return join(formatProgress(d.downloaded, d.total), hostOf(d.url));
      default:
        return d.error ? errorText(d.error) : t(`state.${d.state}` as PlainMessageKey);
    }
  });

  // At rest the right edge says where the download is; hover and focus turn
  // it into the primary action.
  const rest = $derived.by(() => {
    if (running) {
      return pct === null || d.state !== "downloading"
        ? t(`state.${d.state}` as PlainMessageKey)
        : t("percent", { value: pct });
    }
    if (d.state === "completed") return d.completedAt ? formatWhen(d.completedAt, clock.now) : "";
    if (d.state === "queued" && d.startAt !== null) return formatUpcoming(d.startAt, clock.now);
    return t(`state.${d.state}` as PlainMessageKey);
  });

  const actionLabel = $derived(t(`action.${action}` as PlainMessageKey));

  function act(event: MouseEvent) {
    event.stopPropagation();
    const ids = selected ? [...app.selection.ids] : [d.id];
    app.run(action, ids.includes(d.id) ? ids : [d.id]);
  }

  function more(event: MouseEvent) {
    event.stopPropagation();
    onmore(event);
  }
</script>

<!--
  A screen reader hears the name and the state or percentage when the row
  becomes active, then the status line as its description; the pointer's
  buttons are not part of the name, and progress ticks change nothing it
  reads aloud (the list is not a live region).
-->
<div
  id="download-{d.id}"
  class="row"
  class:compact
  class:selected
  class:active
  class:ring
  class:attention
  class:dragged
  data-id={d.id}
  data-state={d.state}
  {draggable}
  role="option"
  aria-selected={selected}
  aria-labelledby="download-{d.id}-name download-{d.id}-rest"
  aria-describedby="download-{d.id}-status"
  aria-posinset={position}
  aria-setsize={count}
>
  <span class="type"><Icon name={icon} /></span>

  <div class="text">
    <span class="name" id="download-{d.id}-name" title={name}>{name}</span>
    {#if !compact}
      <span class="line2">
        {#if d.state !== "completed"}
          <span
            class="bar"
            class:live={running}
            role="progressbar"
            aria-labelledby="download-{d.id}-name"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct ?? undefined}
          >
            <span class="fill" style:transform="scaleX({fraction})"></span>
          </span>
        {/if}
        <span class="status" id="download-{d.id}-status">{status}</span>
      </span>
    {/if}
  </div>

  {#if compact}<span class="status compact-status" id="download-{d.id}-status">{status}</span>{/if}

  <span class="end">
    <span class="rest" id="download-{d.id}-rest" class:glyph={attention || d.state === "paused"}>
      {#if attention}<span class="sign"><Icon name="alert" size={14} /></span>{/if}
      {#if d.state === "paused"}<span class="sign muted"><Icon name="pause" size={14} /></span>{/if}
      {rest}
    </span>
    {#if hovered || active}
      <!-- Created only when needed: scrolling 500 rows creates no buttons. -->
      <span class="actions">
        <IconButton label={actionLabel} tabindex={-1} onclick={act}>
          <Icon name={ACTION_ICONS[action]} />
        </IconButton>
        <IconButton label={t("action.more")} tabindex={-1} onclick={more}>
          <Icon name="more" />
        </IconButton>
      </span>
    {/if}
  </span>

  {#if compact && d.state !== "completed"}
    <span class="underline" class:live={running} aria-hidden="true">
      <span class="fill" style:transform="scaleX({fraction})"></span>
    </span>
  {/if}
</div>

<style>
  .row {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    height: var(--row-height);
    padding: 0 var(--space-2) 0 var(--space-4);
    border-radius: var(--radius-control);
    transition: background-color var(--duration-hover-out) linear;
    outline: none;
  }
  .row:hover {
    background: var(--color-overlay-hover);
    transition: none;
  }
  .row:active {
    background: var(--color-overlay-press);
  }
  /* Selection: an accent tint with a 2 px indicator on the left (#24). */
  .row.selected {
    background: var(--color-accent-tint);
  }
  .row.selected::before {
    content: "";
    position: absolute;
    left: 0;
    top: 8px;
    bottom: 8px;
    width: 2px;
    background: var(--color-accent);
  }
  .row.dragged {
    opacity: 0.45;
  }
  .row.ring {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }

  .type {
    display: inline-flex;
    flex: none;
    color: var(--color-text-subtle);
  }
  .selected .type {
    color: var(--color-text-muted);
  }

  .text {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 3px;
    flex: 1;
    min-width: 0;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    line-height: var(--leading-13);
  }
  .line2 {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    height: var(--leading-12);
  }
  .status {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    font-weight: var(--weight-secondary);
  }
  .compact-status {
    flex: 0 1 auto;
    max-width: 45%;
  }

  /* Chunks are square (#26): the bar and its fill have no radius. */
  .bar {
    position: relative;
    flex: 0 0 clamp(72px, 18%, 160px);
    height: 4px;
    overflow: hidden;
    background: var(--color-progress-track);
  }
  .fill {
    position: absolute;
    inset: 0;
    transform-origin: left center;
    background: var(--color-progress-idle);
    transition: transform var(--duration-tick) var(--ease-data);
  }
  .live .fill {
    background: var(--gradient-progress);
  }

  /* Compact: the fill alone, so a column of rows does not read as ruled lines. */
  .underline {
    position: absolute;
    left: var(--space-4);
    right: var(--space-2);
    bottom: 0;
    height: 2px;
    overflow: hidden;
  }

  .end {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: flex-end;
    flex: none;
    min-width: 72px;
    height: 100%;
  }
  .rest {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--color-text-muted);
    font-size: var(--text-12);
    font-weight: var(--weight-secondary);
    white-space: nowrap;
    padding-right: var(--space-2);
  }
  .sign {
    display: inline-flex;
    color: var(--color-warning);
  }
  [data-state="failed"] .sign,
  [data-state="corrupt"] .sign {
    color: var(--color-error);
  }
  .sign.muted {
    color: var(--color-text-subtle);
  }
  .actions {
    position: absolute;
    right: 0;
    display: flex;
    gap: 2px;
  }

  .end:has(.actions) .rest {
    visibility: hidden;
  }

  @media (forced-colors: active) {
    .row.selected {
      outline: 1px solid Highlight;
    }
    /* The system draws no backgrounds here: outline the bar and keep the
       fill in the highlight colour, so progress stays visible. */
    .bar {
      outline: 1px solid CanvasText;
    }
    .fill {
      forced-color-adjust: none;
      background: Highlight;
    }
  }
</style>
