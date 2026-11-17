<!--
  A playlist's entries in the media sheet (docs/VIDEO.md, "Playlists: as
  built"): place, thumbnail (when the flat entry carries one), title and
  length, with a check for each chosen one. Private and deleted entries are
  shown, disabled. Virtualised, so a playlist of thousands scrolls freely.

  One listbox (aria-multiselectable) with aria-activedescendant, as the main
  list: arrows move, Space chooses, Shift+Space or Shift+click a range (to
  the state of the entry the range starts from), Shift+arrows extend,
  Ctrl+A chooses all; Home, End, Page Up and Page Down move further.
-->
<script lang="ts">
  import Button from "@boltget/ui/components/Button.svelte";
  import { t } from "@boltget/ui/messages";
  import { untrack } from "svelte";
  import { SvelteMap } from "svelte/reactivity";
  import type { MediaEntry } from "../bindings";
  import { downloads } from "../engine";
  import Icon from "../Icon.svelte";
  import { mediaLength } from "./picker";
  import {
    entryLabel,
    moveActive,
    selectAll,
    selectRange,
    selectionText,
    toggle,
  } from "./playlist";

  interface Props {
    entries: MediaEntry[];
    selected: ReadonlySet<number>;
    onselect: (next: Set<number>) => void;
    /** More entries may follow. */
    more: boolean;
    loadingMore: boolean;
    onmore: () => void;
  }

  let { entries, selected, onselect, more, loadingMore, onmore }: Props = $props();

  const ROW = 40;
  const OVERSCAN = 6;
  const PAGE = 6;

  let list: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let viewport = $state(280);
  let active = $state<number | null>(null);
  let anchor = $state<number | null>(null);

  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN));
  const last = $derived(
    Math.min(entries.length, Math.ceil((scrollTop + viewport) / ROW) + OVERSCAN),
  );
  const visible = $derived(entries.slice(first, last));
  const summary = $derived(selectionText(selected, entries));

  // Thumbnails, fetched by the backend for the entries on screen, a few at
  // a time; one that fails is not asked for again.
  const thumbs = new SvelteMap<string, string | null>();
  const MAX_IN_FLIGHT = 4;
  let inFlight = 0;
  /** Bumped when a fetch ends, so the next ones start. */
  let fetched = $state(0);
  $effect(() => {
    void fetched;
    const wanted = visible
      .map((e) => e.thumbnail)
      .filter((url): url is string => !!url && !thumbs.has(url));
    untrack(() => {
      for (const url of wanted) {
        if (inFlight >= MAX_IN_FLIGHT) break;
        thumbs.set(url, null);
        inFlight += 1;
        void downloads.mediaThumbnail(url).then((r) => {
          inFlight -= 1;
          if (r.status === "ok") thumbs.set(url, r.data);
          fetched += 1;
        });
      }
    });
  });

  function reveal(index: number) {
    const at = entries.findIndex((e) => e.index === index);
    if (at < 0 || !list) return;
    const top = at * ROW;
    if (top < list.scrollTop) list.scrollTop = top;
    else if (top + ROW > list.scrollTop + viewport) list.scrollTop = top + ROW - viewport;
  }

  function choose(index: number, range: boolean) {
    if (range) {
      onselect(selectRange(selected, entries, anchor, index));
    } else {
      onselect(toggle(selected, entries, index));
      anchor = index;
    }
    active = index;
  }

  function click(event: MouseEvent) {
    const el = (event.target as HTMLElement).closest<HTMLElement>("[data-index]");
    const entry = entries.find((e) => e.index === Number(el?.dataset.index));
    if (!entry) return;
    list?.focus();
    if (!entry.available) {
      active = entry.index;
      return;
    }
    choose(entry.index, event.shiftKey);
  }

  function keydown(event: KeyboardEvent) {
    const move = (by: number) => {
      event.preventDefault();
      const next = moveActive(entries, active, by);
      if (next === null) return;
      if (event.shiftKey) {
        const from = anchor ?? active ?? next;
        anchor = from;
        onselect(selectRange(selected, entries, from, next));
      }
      active = next;
      reveal(next);
    };
    switch (event.key) {
      case "ArrowDown":
        return move(1);
      case "ArrowUp":
        return move(-1);
      case "PageDown":
        return move(PAGE);
      case "PageUp":
        return move(-PAGE);
      case "Home":
        return move(-entries.length);
      case "End":
        return move(entries.length);
      case " ": {
        event.preventDefault();
        const index = active ?? entries[0]?.index;
        if (index === undefined) return;
        if (entries.find((e) => e.index === index)?.available) choose(index, event.shiftKey);
        else active = index;
        return;
      }
      case "a":
      case "A":
        if (event.ctrlKey || event.metaKey) {
          event.preventDefault();
          onselect(selectAll(entries));
        }
        return;
    }
  }
</script>

<div class="entries">
  <div class="bar">
    <span class="summary" aria-live="polite">{summary}</span>
    <Button variant="ghost" onclick={() => onselect(selectAll(entries))}
      >{t("media.playlist.select_all")}</Button
    >
    <Button variant="ghost" onclick={() => onselect(new Set())}
      >{t("media.playlist.select_none")}</Button
    >
  </div>
  <div
    bind:this={list}
    bind:clientHeight={viewport}
    class="list"
    role="listbox"
    tabindex="0"
    aria-multiselectable="true"
    aria-label={t("media.playlist.entries")}
    aria-describedby="media-entries-keys"
    aria-activedescendant={active !== null ? `media-entry-${active}` : undefined}
    onscroll={() => (scrollTop = list?.scrollTop ?? 0)}
    onkeydown={keydown}
    onclick={click}
    onfocus={() => {
      if (active === null) active = entries[0]?.index ?? null;
    }}
  >
    <div class="track" style:height="{entries.length * ROW}px">
      {#each visible as e, i (e.index)}
        {@const on = selected.has(e.index)}
        {@const thumb = e.thumbnail ? thumbs.get(e.thumbnail) : null}
        <div
          id="media-entry-{e.index}"
          class="entry"
          class:on
          class:active={active === e.index}
          class:gone={!e.available}
          role="option"
          aria-selected={on}
          aria-disabled={!e.available || undefined}
          aria-setsize={entries.length}
          aria-posinset={first + i + 1}
          aria-label={entryLabel(e)}
          title={entryLabel(e)}
          data-index={e.index}
          style:top="{(first + i) * ROW}px"
        >
          <span class="check" aria-hidden="true">
            {#if on}<Icon name="check" size={12} />{/if}
          </span>
          <span class="place">{e.index}</span>
          {#if thumb}
            <img class="thumb" src={thumb} alt="" />
          {:else}
            <span class="thumb empty" aria-hidden="true"><Icon name="video" size={14} /></span>
          {/if}
          <span class="name">{e.title?.trim() || t("media.playlist.untitled")}</span>
          <span class="length">
            {#if !e.available}{t(
                "media.playlist.unavailable",
              )}{:else if e.duration !== null}{mediaLength(e.duration)}{/if}
          </span>
        </div>
      {/each}
    </div>
  </div>
  <div class="foot">
    <p class="hint" id="media-entries-keys">{t("media.playlist.keys")}</p>
    {#if more}
      <Button disabled={loadingMore} onclick={onmore}
        >{loadingMore ? t("media.playlist.loading_more") : t("media.playlist.load_more")}</Button
      >
    {/if}
  </div>
</div>

<style>
  .entries {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .summary {
    flex: 1;
    color: var(--color-text-muted);
    font-size: var(--text-12);
    font-weight: var(--weight-medium);
    font-variant-numeric: tabular-nums;
  }
  .list {
    position: relative;
    height: min(280px, 40vh);
    overflow-y: auto;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-floating);
    background: var(--color-content);
    outline: none;
    user-select: none;
  }
  .list:focus-visible {
    outline: 2px solid var(--color-focus, var(--color-accent));
    outline-offset: 1px;
  }
  .track {
    position: relative;
  }
  .entry {
    position: absolute;
    left: 0;
    right: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 40px;
    padding: 0 var(--space-3);
    border-bottom: 1px solid var(--color-hairline);
    font-size: var(--text-13);
    cursor: default;
  }
  .entry:hover {
    background: var(--color-overlay-hover);
  }
  .entry.on {
    background: var(--color-accent-tint);
  }
  .list:focus-visible .entry.active {
    box-shadow: inset 0 0 0 2px var(--color-accent);
  }
  .entry.gone {
    color: var(--color-text-subtle);
  }
  .check {
    display: inline-grid;
    place-items: center;
    flex: none;
    width: 16px;
    height: 16px;
    border: 1px solid var(--color-control-stroke, var(--color-hairline-strong));
    border-radius: 4px;
    background: var(--color-raised);
    color: var(--color-on-accent, #fff);
  }
  .entry.on .check {
    border-color: var(--color-accent);
    background: var(--color-accent);
  }
  .entry.gone .check {
    opacity: 0.4;
  }
  .place {
    flex: none;
    min-width: 2.5em;
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .thumb {
    flex: none;
    width: 48px;
    height: 27px;
    border-radius: var(--radius-chunk, 2px);
    object-fit: cover;
    background: var(--color-overlay-hover);
  }
  .thumb.empty {
    display: inline-grid;
    place-items: center;
    color: var(--color-text-subtle);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .length {
    flex: none;
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    font-variant-numeric: tabular-nums;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .hint {
    flex: 1;
    margin: 0;
    font-size: var(--text-12);
    line-height: var(--leading-12);
    color: var(--color-text-subtle);
  }

  @media (forced-colors: active) {
    .entry.on .check {
      background: Highlight;
      color: HighlightText;
    }
    .list:focus-visible .entry.active {
      outline: 2px solid Highlight;
    }
  }
</style>
