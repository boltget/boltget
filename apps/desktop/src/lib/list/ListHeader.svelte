<!--
  The view's title and count above the list, with the two view options:
  sorting (on request only, #4) and the details pane (Ctrl+I). Not a toolbar:
  actions on downloads live on the rows, the context menu and the keyboard.
-->
<script lang="ts">
  import IconButton from "@boltget/ui/components/IconButton.svelte";
  import { t, type PlainMessageKey } from "@boltget/ui/messages";
  import { isTauri } from "@tauri-apps/api/core";
  import { app } from "../app.svelte";
  import Icon from "../Icon.svelte";
  import { popupMenu } from "../menu";
  import type { Sort } from "./model";

  interface Props {
    count: number;
  }

  let { count }: Props = $props();

  const SORTS: Sort[] = ["default", "name", "size", "added"];

  const title = $derived.by((): string => {
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
  <span class="options">
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
  .options {
    display: flex;
    gap: 2px;
    margin-left: auto;
  }
</style>
