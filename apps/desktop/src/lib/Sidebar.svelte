<!--
  Views, not a tree (APP-DESIGN #2): All, Active, Completed and Scheduled with
  counts, then the categories by file type, and Settings at the bottom. Under
  900 px of window width it collapses to an icon rail.
-->
<script lang="ts">
  import { t, type PlainMessageKey } from "@boltget/ui/messages";
  import Icon, { type IconName } from "./Icon.svelte";

  interface Entry {
    id: string;
    label: PlainMessageKey;
    icon: IconName;
    count?: number;
  }

  const VIEWS: Entry[] = [
    { id: "all", label: "view.all", icon: "all", count: 0 },
    { id: "active", label: "view.active", icon: "active", count: 0 },
    { id: "completed", label: "view.completed", icon: "completed", count: 0 },
    { id: "scheduled", label: "view.scheduled", icon: "scheduled", count: 0 },
  ];

  const CATEGORIES: Entry[] = [
    { id: "video", label: "category.video", icon: "video" },
    { id: "music", label: "category.music", icon: "music" },
    { id: "archives", label: "category.archives", icon: "archives" },
    { id: "programs", label: "category.programs", icon: "programs" },
    { id: "documents", label: "category.documents", icon: "documents" },
    { id: "other", label: "category.other", icon: "other" },
  ];

  let selected = $state("all");
  const numbers = new Intl.NumberFormat("en");
</script>

{#snippet item(entry: Entry)}
  {@const label = t(entry.label)}
  <li>
    <button
      class="item"
      type="button"
      aria-current={selected === entry.id ? "page" : undefined}
      title={label}
      onclick={() => (selected = entry.id)}
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
    .count {
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
