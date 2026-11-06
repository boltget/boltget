<!--
  The main list (APP-DESIGN #3, #4, #33): one listbox, virtualised with fixed
  heights, grouped in "All", filtered by the view and search. Rows never move
  on their own while the pointer is over the list or the keyboard is working
  in it: sections are held until the user lets go.

  Keyboard (DESIGN-PRINCIPLES #6): arrows, Home, End, PageUp, PageDown, with
  Shift to extend and Ctrl to move without selecting; Ctrl+A; Space pauses or
  resumes; Enter opens; Ctrl+Enter shows in folder; Delete removes (undoable);
  Shift+Delete deletes the file (confirmed); Ctrl+C copies the link; the
  Menu key or Shift+F10 opens the context menu.

  A queue's view is its order (APP-DESIGN, "Queues as built"): rows are
  dragged to a new place (a line shows where they land), and Alt+Up,
  Alt+Down, Alt+Home and Alt+End move the selection. Rows of any view can
  be dragged onto a queue in the sidebar.
-->
<script lang="ts">
  import { t } from "@boltget/ui/messages";
  import { untrack } from "svelte";
  import { app } from "../app.svelte";
  import type { DownloadId } from "../bindings";
  import { clock } from "../clock.svelte";
  import { downloads } from "../engine";
  import { displayName } from "../format";
  import { popupMenu } from "../menu";
  import { dragIds, isDownloadDrag, setDragIds } from "./drag";
  import {
    buildLayout,
    entryAt,
    offsets,
    queueOfView,
    scrollToReveal,
    visibleRange,
    type ListItem,
    type Section,
  } from "./model";
  import { dropBefore, placeOrder, type Shift } from "./queue";
  import Notice from "./Notice.svelte";
  import Row from "./Row.svelte";
  import { rowMenu } from "./rowMenu";
  import {
    click,
    contextClick,
    move,
    prune,
    selectAll,
    selectedInOrder,
    type Move,
  } from "./selection";

  interface Props {
    /** Fired with the layout's row count, for the header. */
    oncount?: (count: number) => void;
  }

  let { oncount }: Props = $props();

  const SECTION_HEIGHT = 36;
  /** Space above the first entry and below the last. */
  const PAD = 8;
  /** How long after the last key or pointer move the list counts as in use. */
  const HOLD_MS = 2500;

  const compact = $derived(app.density === "compact");
  const rowHeight = $derived(compact ? 32 : 40);

  let scroller: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let viewport = $state(0);
  let focused = $state(false);
  let keyboard = $state(false);
  let releaseTimer: ReturnType<typeof setTimeout> | undefined;
  let held = $state.raw<ReadonlyMap<DownloadId, Section> | null>(null);
  let hovered = $state<DownloadId | null>(null);

  // Only membership, state, name, size and dates matter for the layout, so
  // progress ticks never rebuild it.
  const items = $derived.by((): ListItem[] =>
    downloads.downloads
      .filter((d) => !app.hidden.has(d.id))
      .map((d) => ({
        id: d.id,
        state: d.state,
        name: displayName(d.fileName, d.url),
        url: d.url,
        total: d.total,
        createdAt: d.createdAt,
        completedAt: d.completedAt,
        queue: d.queue,
        queuePosition: d.queuePosition,
        startAt: d.startAt,
      })),
  );

  /** The queue whose order this view shows and lets the user change. */
  const ordering = $derived(
    app.sort === "default" && !app.query.trim() ? queueOfView(app.view) : null,
  );
  /** The rows being dragged, and the slot they would land in. */
  let dragging = $state.raw<ReadonlySet<DownloadId> | null>(null);
  let dropSlot = $state<number | null>(null);

  const layout = $derived.by(() => {
    void clock.day;
    return buildLayout(items, {
      view: app.view,
      query: app.query,
      sort: app.sort,
      now: Date.now(),
      held: held ?? undefined,
    });
  });

  const tops = $derived(offsets(layout.entries, rowHeight, SECTION_HEIGHT));
  const total = $derived(tops[tops.length - 1] ?? 0);
  const range = $derived(visibleRange(tops, Math.max(0, scrollTop - PAD), viewport));
  const visible = $derived(layout.entries.slice(range[0], range[1]));
  const order = $derived(layout.order);

  $effect(() => oncount?.(order.length));

  // Rows that left the view (removed, filtered) leave the selection too.
  let previousOrder: DownloadId[] = [];
  $effect(() => {
    const current = order;
    const active = untrack(() => app.selection.active);
    // Removed (or hidden for Undo): follow to the next row. Filtered out by
    // a view or search: just let go.
    const removed = active !== null && (!downloads.get(active) || app.hidden.has(active));
    const next = prune(
      untrack(() => app.selection),
      current,
      previousOrder,
      removed,
    );
    if (next !== app.selection) app.selection = next;
    previousOrder = current;
  });

  // Hold sections while the list is in use, and let go once it is not: the
  // pointer left, or neither it nor the keyboard moved for a while (a pointer
  // resting on the list is not using it).
  function hold(delay = HOLD_MS) {
    if (held === null) held = layout.sections;
    clearTimeout(releaseTimer);
    releaseTimer = setTimeout(() => (held = null), delay);
  }

  function nameOf(id: DownloadId): string {
    const d = downloads.get(id);
    return d ? displayName(d.fileName, d.url) : "";
  }

  function idAt(target: EventTarget | null): DownloadId | null {
    const row = (target as HTMLElement | null)?.closest<HTMLElement>("[data-id]");
    return row ? Number(row.dataset.id) : null;
  }

  function reveal(id: DownloadId | null) {
    if (id === null || !scroller) return;
    const index = layout.entries.findIndex((e) => e.kind === "row" && e.id === id);
    if (index < 0) return;
    // A row right under a section header brings the header along.
    const header = layout.entries[index - 1]?.kind === "section" ? SECTION_HEIGHT : 0;
    const next = scrollToReveal(tops, index, scroller.scrollTop - PAD, viewport, header);
    if (next !== scroller.scrollTop - PAD) scroller.scrollTop = next + PAD;
  }

  function selected(): DownloadId[] {
    return selectedInOrder(app.selection, order);
  }

  function onpointerdown(event: PointerEvent) {
    keyboard = false;
    const id = idAt(event.target);
    if (id === null || event.button !== 0) return;
    // Buttons in the row act on their own row.
    if ((event.target as HTMLElement).closest("button")) return;
    app.selection = click(app.selection, order, id, {
      shift: event.shiftKey,
      ctrl: event.ctrlKey || event.metaKey,
    });
    scroller?.focus({ preventScroll: true });
  }

  function ondblclick(event: MouseEvent) {
    const id = idAt(event.target);
    if (id === null || (event.target as HTMLElement).closest("button")) return;
    const d = downloads.get(id);
    if (!d) return;
    if (d.state === "completed") app.run("open", [id]);
    else app.togglePane(true);
  }

  async function openMenu(at?: { x: number; y: number }) {
    const ids = selected();
    if (ids.length) await popupMenu(rowMenu(ids), at);
  }

  function oncontextmenu(event: MouseEvent) {
    event.preventDefault();
    const id = idAt(event.target);
    if (id === null) return;
    app.selection = contextClick(app.selection, id);
    scroller?.focus({ preventScroll: true });
    void openMenu();
  }

  function onmore(id: DownloadId, event: MouseEvent) {
    app.selection = contextClick(app.selection, id);
    const button = (event.currentTarget as HTMLElement | null)?.getBoundingClientRect();
    void openMenu(button ? { x: button.left, y: button.bottom + 2 } : undefined);
  }

  function menuAtActive() {
    const id = app.selection.active;
    const row = id === null ? null : document.getElementById(`download-${id}`);
    const box = row?.getBoundingClientRect();
    void openMenu(box ? { x: box.left + 48, y: box.top + box.height / 2 } : undefined);
  }

  function step(by: Move, event: KeyboardEvent) {
    event.preventDefault();
    app.selection = move(app.selection, order, by, {
      shift: event.shiftKey,
      ctrl: event.ctrlKey,
    });
    reveal(app.selection.active);
  }

  const SHIFTS: Record<string, Shift> = {
    ArrowUp: "up",
    ArrowDown: "down",
    Home: "top",
    End: "bottom",
  };

  function onkeydown(event: KeyboardEvent) {
    keyboard = true;
    hold();
    const page = Math.max(1, Math.floor(viewport / rowHeight) - 1);
    const ctrl = event.ctrlKey || event.metaKey;
    const shift = SHIFTS[event.key];
    if (event.altKey && !ctrl && !event.shiftKey && shift) {
      // Alt+arrows move the selection in its queue (in a queue's view).
      event.preventDefault();
      if (ordering !== null) {
        app.shift(selected(), shift, app.selection.active ?? undefined);
        queueMicrotask(() => reveal(app.selection.active));
      }
      return;
    }
    switch (event.key) {
      case "ArrowDown":
        return step(1, event);
      case "ArrowUp":
        return step(-1, event);
      case "PageDown":
        return step(page, event);
      case "PageUp":
        return step(-page, event);
      case "Home":
        return step("home", event);
      case "End":
        return step("end", event);
      case " ":
        event.preventDefault();
        if (ctrl && app.selection.active !== null) {
          app.selection = click(app.selection, order, app.selection.active, { ctrl: true });
        } else app.toggle(selected());
        return;
      case "Enter": {
        event.preventDefault();
        const ids = selected();
        if (ctrl) app.run("show", ids);
        else if (ids.length === 1 && downloads.get(ids[0] as DownloadId)?.state === "completed") {
          app.run("open", ids);
        } else app.togglePane(true);
        return;
      }
      case "Delete":
        event.preventDefault();
        app.run(event.shiftKey ? "delete" : "remove", selected());
        return;
      case "ContextMenu":
        event.preventDefault();
        menuAtActive();
        return;
      case "F10":
        if (event.shiftKey) {
          event.preventDefault();
          menuAtActive();
        }
        return;
    }
    if (ctrl && !event.shiftKey && !event.altKey) {
      if (event.code === "KeyA") {
        event.preventDefault();
        app.selection = selectAll(app.selection, order);
      } else if (event.code === "KeyC") {
        event.preventDefault();
        app.run("copy", selected());
      }
    }
  }

  function onfocus() {
    focused = true;
    // Tabbing in lands on the selection, or the first row.
    if (app.selection.active === null && order.length) {
      const first = selected()[0] ?? (order[0] as DownloadId);
      app.selection = { ...app.selection, active: first, anchor: first };
    }
  }

  // Dragging rows: in a queue's view to a new place, anywhere onto a queue
  // in the sidebar.
  function ondragstart(event: DragEvent) {
    const id = idAt(event.target);
    if (id === null || !event.dataTransfer) return;
    const ids = app.selection.ids.has(id) ? selected() : [id];
    setDragIds(event.dataTransfer, ids);
    dragging = new Set(ids);
  }

  function slotAt(event: DragEvent): number | null {
    if (!scroller) return null;
    const box = scroller.getBoundingClientRect();
    const y = event.clientY - box.top + scroller.scrollTop - PAD;
    if (layout.entries.length === 0) return 0;
    const index = entryAt(tops, Math.max(0, y));
    const entry = layout.entries[index];
    if (!entry || entry.kind !== "row") return null;
    const top = tops[index] ?? 0;
    const bottom = tops[index + 1] ?? top;
    const slot = entry.index + (y > (top + bottom) / 2 ? 1 : 0);
    return Math.min(slot, order.length);
  }

  function ondragover(event: DragEvent) {
    if (ordering === null || !isDownloadDrag(event.dataTransfer)) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
    dropSlot = slotAt(event);
  }

  function ondrop(event: DragEvent) {
    const queue = ordering;
    const slot = dropSlot;
    const moving = dragging;
    dropSlot = null;
    if (queue === null || slot === null || !isDownloadDrag(event.dataTransfer)) return;
    event.preventDefault();
    event.stopPropagation();
    const ids = moving ? order.filter((id) => moving.has(id)) : dragIds(event.dataTransfer);
    if (ids.length === 0) return;
    const before = dropBefore(order, slot, new Set(ids));
    const next = placeOrder(order, ids, before);
    if (next.every((id, i) => id === order[i])) return;
    void downloads.moveDownloads(ids, queue, before, order);
    app.announce(
      t("announce.moved", {
        count: ids.length,
        name: nameOf(ids[0] as DownloadId),
        place: next.indexOf(ids[0] as DownloadId) + 1,
        total: next.length,
      }),
    );
  }

  function ondragend() {
    dragging = null;
    dropSlot = null;
  }

  /** Where the drop line is drawn. */
  const dropTop = $derived.by(() => {
    if (dropSlot === null) return null;
    const row = layout.entries.findIndex((e) => e.kind === "row" && e.index === dropSlot);
    const y = row >= 0 ? (tops[row] ?? 0) : total;
    return y + PAD;
  });

  // A view with nothing in it says why, in one line.
  const emptyText = $derived.by(() => {
    if (app.query.trim()) return t("empty.search", { query: app.query.trim() });
    const queue = queueOfView(app.view);
    if (queue !== null) return t("empty.view.queue", { queue: app.queueName(queue) });
    switch (app.view) {
      case "all":
        return "";
      case "active":
      case "completed":
      case "scheduled":
        return t(`empty.view.${app.view}`);
      default:
        return t("empty.view.category", {
          category: t(
            `category.${app.view as Exclude<typeof app.view, `queue:${number}`>}`,
          ).toLowerCase(),
        });
    }
  });

  const activeId = $derived(
    app.selection.active !== null && order.includes(app.selection.active)
      ? `download-${app.selection.active}`
      : undefined,
  );
</script>

{#if order.length === 0}
  <Notice title={emptyText} />
{/if}
<div
  bind:this={scroller}
  class="scroller"
  class:hidden={order.length === 0}
  style:--row-height="{rowHeight}px"
  role="listbox"
  tabindex="0"
  aria-label={t("list.label")}
  aria-multiselectable="true"
  aria-activedescendant={activeId}
  bind:clientHeight={viewport}
  onscroll={() => {
    scrollTop = scroller?.scrollTop ?? 0;
    hold();
  }}
  onpointermove={() => hold()}
  onpointerover={(event) => (hovered = idAt(event.target))}
  onpointerleave={() => {
    hovered = null;
    hold(keyboard ? HOLD_MS : 300);
  }}
  {onpointerdown}
  {ondblclick}
  {oncontextmenu}
  {onkeydown}
  {onfocus}
  {ondragstart}
  {ondragover}
  {ondrop}
  {ondragend}
  ondragleave={(event) => {
    if (!scroller?.contains(event.relatedTarget as Node | null)) dropSlot = null;
  }}
  onblur={() => (focused = false)}
>
  <div class="spacer" style:height="{total + 2 * PAD}px">
    {#if dropTop !== null}
      <div class="drop-line" style:transform="translateY({dropTop - 1}px)" aria-hidden="true"></div>
    {/if}
    {#each visible as entry, i (entry.key)}
      {@const index = range[0] + i}
      <div class="slot" style:transform="translateY({(tops[index] ?? 0) + PAD}px)">
        {#if entry.kind === "section"}
          <div class="section" role="presentation">
            <span>{t(`section.${entry.section}`)}</span>
            <span class="count">{entry.count}</span>
          </div>
        {:else}
          {@const d = downloads.get(entry.id)}
          {#if d}
            <Row
              draggable
              dragged={dragging?.has(d.id) ?? false}
              download={d}
              selected={app.selection.ids.has(d.id)}
              active={app.selection.active === d.id}
              hovered={hovered === d.id}
              ring={focused && keyboard && app.selection.active === d.id}
              {compact}
              position={entry.index + 1}
              count={order.length}
              onmore={(event) => onmore(d.id, event)}
            />
          {/if}
        {/if}
      </div>
    {/each}
  </div>
</div>

<style>
  .scroller {
    position: relative;
    height: 100%;
    overflow-y: auto;
    overflow-x: hidden;
    contain: strict;
    outline: none;
  }
  .scroller:focus-visible {
    outline: none;
  }
  .scroller.hidden {
    display: none;
  }
  .spacer {
    position: relative;
  }
  .slot {
    position: absolute;
    top: 0;
    left: var(--space-2);
    right: var(--space-2);
    will-change: transform;
  }
  .drop-line {
    position: absolute;
    top: 0;
    left: var(--space-4);
    right: var(--space-4);
    z-index: 2;
    height: 2px;
    background: var(--color-accent);
    pointer-events: none;
  }
  .section {
    display: flex;
    align-items: flex-end;
    gap: var(--space-2);
    height: 36px;
    padding: 0 var(--space-4) var(--space-2);
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    font-weight: var(--weight-medium);
  }
  .count {
    font-weight: var(--weight-secondary);
    opacity: 0.8;
  }
</style>
