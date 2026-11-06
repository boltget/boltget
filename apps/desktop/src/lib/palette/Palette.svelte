<!--
  The command palette (Ctrl+K, APP-DESIGN #20): one field over every action,
  setting and download. Typing ranks them (fuzzy, with synonyms); arrows move,
  Enter runs, Tab opens the result in context (the setting's row, the
  download in the list), Esc closes. Each result shows its shortcut.

  A native modal <dialog>: focus stays in the field, which is a combobox
  over a listbox (aria-activedescendant), so a screen reader hears the
  active result as it moves.
-->
<script lang="ts">
  import Keycap from "@boltget/ui/components/Keycap.svelte";
  import { t, type PlainMessageKey } from "@boltget/ui/messages";
  import { untrack } from "svelte";
  import { app } from "../app.svelte";
  import { downloads } from "../engine";
  import { displayName } from "../format";
  import Icon from "../Icon.svelte";
  import { openAddFromClipboard } from "../paste";
  import {
    buildEntries,
    GROUPS,
    initialEntries,
    step,
    type PaletteActions,
    type PaletteEntry,
  } from "./entries";
  import { rank } from "./fuzzy";

  /** Results shown at most while typing. */
  const LIMIT = 60;
  const EXIT_MS = 100;

  let dialog: HTMLDialogElement | undefined = $state();
  let field: HTMLInputElement | undefined = $state();
  let query = $state("");
  let active = $state(0);
  let closing = $state(false);

  const reduced =
    typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

  const actions: PaletteActions = {
    add: () => {
      app.closeSettings();
      void openAddFromClipboard();
    },
    pauseAll: () => app.pauseAll(),
    resumeAll: () => app.resumeAll(),
    settings: (row) => app.openSettings(row),
    search: () => {
      // The title bar's field: it searches downloads, or settings in Settings.
      queueMicrotask(() => document.querySelector<HTMLInputElement>("#titlebar-search")?.focus());
    },
    details: () => {
      app.closeSettings();
      app.togglePane();
    },
    density: () => app.setDensity(app.density === "compact" ? "comfortable" : "compact"),
    view: (view) => app.showView(view),
    limit: (bps) => void downloads.setSpeedLimit({ kind: "global" }, bps),
    folder: () => void downloads.openDownloadsFolder(),
    quit: () => void downloads.quit(),
    update: (patch) =>
      void downloads.updateSettings(patch).then((result) => {
        if (result.status === "error") app.openSettings();
      }),
    act: (action, ids) => app.run(action, ids),
    reveal: (id) => app.reveal(id),
    toggleQueue: (id) => app.toggleQueue(id),
    editQueue: (id) => app.editQueue(id),
    startNow: (ids) => app.startNow(ids),
    startLater: (ids) => app.startLater(ids),
    moveTo: (ids, queue) => app.moveTo(ids, queue),
  };

  const entries = $derived.by(() => {
    if (!app.palette) return [];
    const toItem = (d: (typeof downloads.downloads)[number]) => ({
      id: d.id,
      name: displayName(d.fileName, d.url),
      state: d.state,
      queue: d.queue,
    });
    const visible = downloads.downloads.filter((d) => !app.hidden.has(d.id));
    return buildEntries({
      settings: downloads.settings,
      compact: app.density === "compact",
      selection: visible.filter((d) => app.selection.ids.has(d.id)).map(toItem),
      downloads: visible.map(toItem),
      queues: downloads.queues.map((q) => ({
        id: q.id,
        name: app.queueName(q),
        started: q.started,
      })),
      stateLabel: (s) => t(`state.${s}` as PlainMessageKey),
      do: actions,
    });
  });

  const typing = $derived(query.trim().length > 0);
  const results = $derived(
    typing
      ? rank(entries, query, (e) => ({ label: e.label, synonyms: e.synonyms }), LIMIT)
      : initialEntries(entries),
  );
  /** Grouped while nothing is typed; one ranked list while typing. */
  const groups = $derived.by(() => {
    if (typing) return [{ id: "results", label: null, entries: results }];
    return GROUPS.map((g) => ({
      id: g.id,
      label: t(g.label),
      entries: results.filter((e) => e.group === g.id),
    })).filter((g) => g.entries.length > 0);
  });
  const ordered = $derived(groups.flatMap((g) => g.entries));
  const current = $derived(ordered[active]);

  const optionId = (i: number) => `palette-option-${i}`;

  // Opening and closing ----------------------------------------------------

  $effect(() => {
    const open = app.palette;
    untrack(() => {
      if (open && !dialog?.open) {
        query = "";
        active = 0;
        closing = false;
        dialog?.showModal();
        field?.focus();
      } else if (!open && dialog?.open) {
        closing = true;
        setTimeout(
          () => {
            dialog?.close();
            closing = false;
          },
          reduced ? 0 : EXIT_MS,
        );
      }
    });
  });

  // A new query starts at the best result.
  $effect(() => {
    void query;
    active = 0;
  });

  // Keep the active result in view.
  $effect(() => {
    const i = active;
    if (!app.palette) return;
    queueMicrotask(() =>
      document.getElementById(optionId(i))?.scrollIntoView({ block: "nearest" }),
    );
  });

  function close() {
    app.palette = false;
  }

  function run(entry: PaletteEntry | undefined, inContext = false) {
    if (!entry) return;
    close();
    // After the dialog has let focus go, so what runs can take it.
    queueMicrotask(() => (inContext && entry.reveal ? entry.reveal() : entry.run()));
  }

  function onkeydown(event: KeyboardEvent) {
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        active = step(active, 1, ordered.length);
        break;
      case "ArrowUp":
        event.preventDefault();
        active = step(active, -1, ordered.length);
        break;
      case "PageDown":
        event.preventDefault();
        active = Math.min(ordered.length - 1, active + 8);
        break;
      case "PageUp":
        event.preventDefault();
        active = Math.max(0, active - 8);
        break;
      case "Enter":
        if (event.isComposing) return;
        event.preventDefault();
        run(current);
        break;
      case "Tab":
        event.preventDefault();
        if (!event.shiftKey) run(current, true);
        break;
    }
  }

  function oncancel(event: Event) {
    event.preventDefault();
    close();
  }

  function onclick(event: MouseEvent) {
    // A click on the backdrop (the dialog itself, outside the box) closes.
    if (event.target === dialog) close();
  }
</script>

<dialog
  bind:this={dialog}
  class="palette"
  class:closing
  class:reduced
  aria-label={t("palette.label")}
  {oncancel}
  {onclick}
>
  <div class="box">
    <div class="search">
      <Icon name="search" />
      <input
        bind:this={field}
        bind:value={query}
        {onkeydown}
        type="text"
        role="combobox"
        aria-expanded="true"
        aria-controls="palette-list"
        aria-autocomplete="list"
        aria-activedescendant={current ? optionId(active) : undefined}
        aria-label={t("palette.label")}
        placeholder={t("palette.placeholder")}
        spellcheck="false"
        autocomplete="off"
      />
      <Keycap shortcut={t("shortcut.close")} />
    </div>

    <div class="results" id="palette-list" role="listbox" aria-label={t("palette.label")}>
      {#each groups as group (group.id)}
        <div role="group" aria-label={group.label ?? t("palette.label")}>
          {#if group.label}<div class="group" aria-hidden="true">{group.label}</div>{/if}
          {#each group.entries as entry (entry.id)}
            {@const index = ordered.indexOf(entry)}
            <div
              id={optionId(index)}
              class="option"
              role="option"
              tabindex="-1"
              aria-selected={index === active}
              onmousemove={() => (active = index)}
              onclick={() => run(entry)}
              onkeydown={() => {}}
            >
              <span class="icon"><Icon name={entry.icon} /></span>
              <span class="label">{entry.label}</span>
              {#if entry.hint}<span class="hint">{entry.hint}</span>{/if}
              {#if entry.shortcut}<Keycap shortcut={entry.shortcut} />{/if}
            </div>
          {/each}
        </div>
      {/each}
      {#if ordered.length === 0}
        <p class="none">{t("palette.none", { query: query.trim() })}</p>
      {/if}
    </div>

    <div class="footer" aria-hidden="true">
      <span><Keycap shortcut={t("shortcut.enter")} />{t("palette.hint.run")}</span>
      <span><Keycap shortcut={t("shortcut.tab")} />{t("palette.hint.open")}</span>
      <span><Keycap shortcut={t("shortcut.close")} />{t("palette.hint.close")}</span>
    </div>
    <div class="visually-hidden" aria-live="polite">
      {typing ? t("palette.results", { count: ordered.length }) : ""}
    </div>
  </div>
</dialog>

<style>
  .palette {
    width: min(620px, calc(100vw - 48px));
    max-width: none;
    max-height: none;
    margin: 12vh auto 0;
    padding: 0;
    border: 0;
    border-radius: var(--radius-floating);
    background: var(--color-raised);
    color: var(--color-text);
    box-shadow: var(--elevation-floating);
    overflow: hidden;
    animation: enter var(--duration-small) var(--ease-enter);
  }
  .palette::backdrop {
    background: var(--color-scrim);
    animation: fade var(--duration-small) var(--ease-enter);
  }
  .palette.closing {
    opacity: 0;
    transition: opacity var(--duration-small-exit) var(--ease-exit);
  }
  .palette.reduced,
  .palette.reduced::backdrop {
    animation: none;
  }
  @keyframes enter {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }

  .box {
    display: flex;
    flex-direction: column;
    max-height: min(520px, 70vh);
  }
  .search {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    height: 48px;
    padding: 0 var(--space-4);
    border-bottom: 1px solid var(--color-hairline);
    color: var(--color-text-subtle);
  }
  .search input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: 0;
    background: transparent;
    color: var(--color-text);
    font-size: var(--text-15);
    outline: none;
  }
  .search input::placeholder {
    color: var(--color-text-subtle);
  }

  .results {
    flex: 1;
    min-height: 0;
    padding: var(--space-1) var(--space-2) var(--space-2);
    overflow-y: auto;
  }
  .group {
    padding: var(--space-3) var(--space-2) var(--space-1);
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    font-weight: var(--weight-medium);
  }
  .option {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    height: 36px;
    padding: 0 var(--space-2);
    border-radius: var(--radius-control);
    color: var(--color-text);
    cursor: default;
  }
  .option[aria-selected="true"] {
    background: var(--color-accent-tint);
  }
  .option[aria-selected="true"]::before {
    content: "";
    position: absolute;
    left: 0;
    top: 9px;
    bottom: 9px;
    width: 2px;
    background: var(--color-accent);
  }
  .icon {
    display: inline-flex;
    color: var(--color-text-muted);
  }
  .option[aria-selected="true"] .icon {
    color: var(--color-accent);
  }
  .label {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .option :global(.keycaps) {
    margin-left: auto;
    flex: none;
  }
  .none {
    margin: 0;
    padding: var(--space-6) var(--space-3);
    color: var(--color-text-muted);
    text-align: center;
  }

  .footer {
    display: flex;
    gap: var(--space-4);
    padding: var(--space-2) var(--space-4);
    border-top: 1px solid var(--color-hairline);
    color: var(--color-text-subtle);
    font-size: var(--text-12);
  }
  .footer span {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  @media (forced-colors: active) {
    .option[aria-selected="true"] {
      outline: 1px solid Highlight;
    }
  }
</style>
