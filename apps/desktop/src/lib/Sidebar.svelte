<!--
  Views, not a tree (APP-DESIGN #2): All, Active, Completed and Scheduled with
  counts, then the categories by file type, then the queues with their
  counts (APP-DESIGN, "Queues as built"), and Settings at the bottom. Under
  900 px of window width it collapses to an icon rail.

  A queue takes downloads dropped on it (dragged from the list), and its
  context menu starts or stops it, opens its settings or deletes it.
-->
<script lang="ts">
  import IconButton from "@boltget/ui/components/IconButton.svelte";
  import { t, type PlainMessageKey } from "@boltget/ui/messages";
  import { app } from "./app.svelte";
  import type { QueueInfo } from "./bindings";
  import { downloads } from "./engine";
  import Icon, { type IconName } from "./Icon.svelte";
  import { dragIds, isDownloadDrag } from "./list/drag";
  import { queueView, viewCounts, type View } from "./list/model";
  import { queueCounts } from "./list/queue";
  import { popupMenu } from "./menu";

  interface Entry {
    id: View | "settings";
    label: PlainMessageKey;
    icon: IconName;
    count?: number;
  }

  // Counts read only each download's state, so progress never touches them.
  const counts = $derived(viewCounts(downloads.downloads.filter((d) => !app.hidden.has(d.id))));

  const VIEWS: Entry[] = $derived([
    { id: "all", label: "view.all", icon: "all", count: counts.all },
    { id: "active", label: "view.active", icon: "active", count: counts.active },
    { id: "completed", label: "view.completed", icon: "completed", count: counts.completed },
    { id: "scheduled", label: "view.scheduled", icon: "scheduled", count: counts.scheduled },
  ]);

  const CATEGORIES: Entry[] = [
    { id: "video", label: "category.video", icon: "video" },
    { id: "music", label: "category.music", icon: "music" },
    { id: "archives", label: "category.archives", icon: "archives" },
    { id: "programs", label: "category.programs", icon: "programs" },
    { id: "documents", label: "category.documents", icon: "documents" },
    { id: "other", label: "category.other", icon: "other" },
  ];

  const numbers = new Intl.NumberFormat("en");

  // Counts read only each download's state and queue.
  const perQueue = $derived(queueCounts(downloads.downloads.filter((d) => !app.hidden.has(d.id))));
  /** The queue a drag is over, for its highlight. */
  let dropping = $state<number | null>(null);

  function choose(id: Entry["id"]) {
    if (id === "settings") app.openSettings();
    else app.view = id;
  }

  function queueMenu(q: QueueInfo, event: MouseEvent) {
    event.preventDefault();
    void popupMenu([
      {
        text: t(q.started ? "queue.stop" : "queue.start"),
        run: () => app.toggleQueue(q.id),
      },
      { text: t("queue.settings"), run: () => app.editQueue(q.id) },
      ...(q.isDefault
        ? []
        : [
            "separator" as const,
            {
              text: t("queue.delete"),
              run: () => {
                const name = app.queueName(q);
                void downloads.deleteQueue(q.id).then((r) => {
                  if (r.status === "ok") {
                    if (app.view === queueView(q.id)) app.view = queueView(1);
                    app.toast(t("queue.deleted", { name, main: t("queue.main") }));
                  }
                });
              },
            },
          ]),
    ]);
  }

  function ondragover(q: QueueInfo, event: DragEvent) {
    if (!isDownloadDrag(event.dataTransfer)) return;
    event.preventDefault();
    if (event.dataTransfer) event.dataTransfer.dropEffect = "move";
    dropping = q.id;
  }

  function ondrop(q: QueueInfo, event: DragEvent) {
    dropping = null;
    if (!isDownloadDrag(event.dataTransfer)) return;
    event.preventDefault();
    event.stopPropagation();
    app.moveTo(dragIds(event.dataTransfer), q.id);
  }
</script>

{#snippet item(entry: Entry)}
  {@const label = t(entry.label)}
  <li>
    <button
      class="item"
      type="button"
      aria-current={app.view === entry.id ? "page" : undefined}
      title={label}
      onclick={() => choose(entry.id)}
    >
      <Icon name={entry.icon} />
      <span class="label">{label}</span>
      {#if entry.count !== undefined}
        <span class="count" aria-label={t("view.count", { count: entry.count })}
          >{numbers.format(entry.count)}</span
        >
      {/if}
    </button>
  </li>
{/snippet}

<nav class="sidebar" aria-label={t("sidebar.label")}>
  <ul class="group" aria-label={t("sidebar.views")}>
    {#each VIEWS as entry (entry.id)}{@render item(entry)}{/each}
  </ul>

  <h2 class="heading" id="sidebar-categories">{t("sidebar.categories")}</h2>
  <ul class="group" aria-labelledby="sidebar-categories">
    {#each CATEGORIES as entry (entry.id)}{@render item(entry)}{/each}
  </ul>

  <div class="heading-row">
    <h2 class="heading" id="sidebar-queues">{t("sidebar.queues")}</h2>
    <span class="add-queue">
      <IconButton label={t("queue.new")} onclick={() => app.editQueue(null)}>
        <Icon name="plus" size={14} />
      </IconButton>
    </span>
  </div>
  <ul class="group" aria-labelledby="sidebar-queues">
    {#each downloads.queues as q (q.id)}
      {@const name = app.queueName(q)}
      {@const count = perQueue.get(q.id) ?? 0}
      {@const view = queueView(q.id)}
      <li>
        <button
          class="item"
          class:drop={dropping === q.id}
          type="button"
          aria-current={app.view === view ? "page" : undefined}
          title={q.started ? name : `${name} · ${t("queue.stopped")}`}
          onclick={() => (app.view = view)}
          oncontextmenu={(event) => queueMenu(q, event)}
          ondragover={(event) => ondragover(q, event)}
          ondragleave={() => (dropping = null)}
          ondrop={(event) => ondrop(q, event)}
        >
          <Icon name="queue" />
          <span class="label">{name}</span>
          {#if !q.started}
            <span class="stopped" aria-label={t("queue.stopped")}
              ><Icon name="pause" size={12} /></span
            >
          {/if}
          <span class="count" aria-label={t("view.count", { count })}>{numbers.format(count)}</span>
        </button>
      </li>
    {/each}
  </ul>

  <ul class="group footer">
    {@render item({ id: "settings", label: "sidebar.settings", icon: "settings" })}
  </ul>
</nav>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    width: var(--sidebar-width);
    padding: var(--space-1) var(--space-2) var(--space-2);
    background: var(--color-chrome);
    overflow-y: auto;
  }
  .group {
    display: flex;
    flex-direction: column;
    gap: 1px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .footer {
    margin-top: auto;
    padding-top: var(--space-2);
  }
  .heading-row {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
  }
  .heading-row .heading {
    flex: 1;
  }
  .add-queue {
    margin: 0 var(--space-1) 2px 0;
  }
  .stopped {
    display: inline-flex;
    color: var(--color-text-subtle);
  }
  .item.drop {
    background: var(--color-accent-tint);
    outline: 1px dashed var(--color-accent);
    outline-offset: -1px;
  }
  .heading {
    margin: var(--space-5) 0 var(--space-1);
    padding: 0 var(--space-3);
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    font-weight: var(--weight-medium);
  }

  .item {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    height: var(--control-height-lg);
    padding: 0 var(--space-3);
    border: 0;
    border-radius: var(--radius-control);
    background: transparent;
    color: var(--color-text-muted);
    text-align: left;
    transition: background-color var(--duration-hover-out) linear;
  }
  .item:hover {
    background: var(--color-overlay-hover);
    color: var(--color-text);
    transition: none;
  }
  .item:active {
    background: var(--color-overlay-press);
    transition: none;
  }
  /* Selection: an accent tint with a 2 px indicator on the left (#24). */
  .item[aria-current="page"] {
    background: var(--color-accent-tint);
    color: var(--color-text);
    font-weight: var(--weight-medium);
  }
  .item[aria-current="page"]::before {
    content: "";
    position: absolute;
    left: 0;
    top: 8px;
    bottom: 8px;
    width: 2px;
    border-radius: var(--radius-chunk);
    background: var(--color-accent);
  }
  .item[aria-current="page"] :global(.icon) {
    color: var(--color-accent);
  }
  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    font-weight: var(--weight-secondary);
    font-variant-numeric: tabular-nums;
  }

  @media (forced-colors: active) {
    .item[aria-current="page"] {
      outline: 1px solid Highlight;
    }
  }

  /* The icon rail. */
  @media (max-width: 899px) {
    .sidebar {
      width: var(--sidebar-rail-width);
      padding-inline: var(--space-1);
    }
    .item {
      justify-content: center;
      padding: 0;
    }
    .label,
    .count,
    .add-queue,
    .stopped {
      display: none;
    }
    .heading {
      height: 1px;
      margin: var(--space-3) var(--space-2);
      padding: 0;
      overflow: hidden;
      background: var(--color-hairline);
      color: transparent;
    }
  }
</style>
