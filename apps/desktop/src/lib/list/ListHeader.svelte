<!--
  The view's title and count above the list, with the two view options:
  sorting (on request only, #4) and the details pane (Ctrl+I). Not a toolbar:
  actions on downloads live on the rows, the context menu and the keyboard.

  A queue's view adds what concerns the queue itself: how it runs (its
  limit, daily times and finish action, in one muted line), Start or Stop,
  and its settings.
-->
<script lang="ts">
  import Button from "@boltget/ui/components/Button.svelte";
  import IconButton from "@boltget/ui/components/IconButton.svelte";
  import { t, type PlainMessageKey } from "@boltget/ui/messages";
  import { isTauri } from "@tauri-apps/api/core";
  import { app } from "../app.svelte";
  import { downloads } from "../engine";
  import Icon from "../Icon.svelte";
  import { popupMenu } from "../menu";
  import { queueOfView, type Sort } from "./model";
  import { timeValue } from "./queue";

  interface Props {
    count: number;
  }

  let { count }: Props = $props();

  const SORTS: Sort[] = ["default", "name", "size", "added"];

  const queue = $derived.by(() => {
    const id = queueOfView(app.view);
    return id === null ? undefined : downloads.queue(id);
  });

  /** How the queue runs, in one line. */
  const how = $derived.by((): string => {
    if (!queue) return "";
    const parts = [
      queue.started
        ? t("queue.status.started", { count: queue.maxConcurrent })
        : t("queue.status.stopped"),
    ];
    const start = timeValue(queue.startTime);
    const stop = timeValue(queue.stopTime);
    if (start && stop) parts.push(t("queue.status.times", { start, stop }));
    else if (start) parts.push(t("queue.status.start_time", { start }));
    else if (stop) parts.push(t("queue.status.stop_time", { stop }));
    if (queue.onFinish !== "nothing") parts.push(t(`queue.status.finish.${queue.onFinish}`));
    return parts.join(" · ");
  });

  const title = $derived.by((): string => {
    if (queue) return app.queueName(queue);
    if (queueOfView(app.view) !== null) return "";
    const key = (
      ["all", "active", "completed", "scheduled"].includes(app.view)
        ? `view.${app.view}`
        : `category.${app.view}`
    ) as PlainMessageKey;
    return t(key);
  });

  function sortMenu(event: MouseEvent) {
    if (!isTauri()) {
      // The UI alone in a browser has no native menus: cycle instead.
      app.sort = SORTS[(SORTS.indexOf(app.sort) + 1) % SORTS.length] ?? "default";
      return;
    }
    const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
    void popupMenu(
      [
        ...SORTS.map((sort) => ({
          text: t(`sort.${sort}`),
          checked: app.sort === sort,
          run: () => (app.sort = sort),
        })),
        "separator" as const,
        {
          text: t("density.compact"),
          checked: app.density === "compact",
          accelerator: "Ctrl+Shift+D",
          run: () => app.setDensity(app.density === "compact" ? "comfortable" : "compact"),
        },
      ],
      { x: box.left, y: box.bottom + 4 },
    );
  }
</script>

<header class="head">
  <h1 class="title">{title}</h1>
  <span class="count" aria-label={t("list.count", { count })}>{count}</span>
  {#if queue}
    <span class="how">{how}</span>
  {/if}
  <span class="options">
    {#if queue}
      <Button variant="ghost" onclick={() => app.toggleQueue(queue.id)}>
        {#snippet icon()}<Icon name={queue.started ? "stop" : "play"} size={14} />{/snippet}
        {t(queue.started ? "queue.stop" : "queue.start")}
      </Button>
      <IconButton label={t("queue.settings")} onclick={() => app.editQueue(queue.id)}>
        <Icon name="settings" />
      </IconButton>
      <span class="divider" aria-hidden="true"></span>
    {/if}
    <IconButton
      label={app.sort === "default"
        ? t("sort.label")
        : t("sort.current", { sort: t(`sort.${app.sort}`) })}
      pressed={app.sort !== "default"}
      onclick={sortMenu}
    >
      <Icon name="sort" />
    </IconButton>
    <IconButton
      label={app.paneOpen ? t("pane.hide") : t("pane.show")}
      pressed={app.paneOpen}
      onclick={() => app.togglePane()}
    >
      <Icon name="pane" />
    </IconButton>
  </span>
</header>

<style>
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 48px;
    padding: 0 var(--space-3) 0 var(--space-6);
    flex: none;
  }
  .title {
    margin: 0;
    font-size: var(--text-15);
    line-height: var(--leading-15);
    font-weight: var(--weight-title);
    letter-spacing: -0.005em;
  }
  .count {
    color: var(--color-text-subtle);
    font-size: var(--text-13);
  }
  .how {
    min-width: 0;
    margin-left: var(--space-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    font-weight: var(--weight-secondary);
  }
  .options {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: none;
    margin-left: auto;
  }
  .divider {
    width: 1px;
    height: 16px;
    margin: 0 var(--space-1);
    background: var(--color-hairline);
  }
</style>
