<!--
  The title bar is the only chrome (APP-DESIGN #1, #31): the mark and name,
  search, the one primary action and the caption buttons. Everything that is
  not a control drags the window; double-click maximizes.
-->
<script lang="ts">
  import Button from "@boltget/ui/components/Button.svelte";
  import Keycap from "@boltget/ui/components/Keycap.svelte";
  import { t } from "@boltget/ui/messages";
  import { onMount } from "svelte";
  import icon16 from "../../../../brand/svg/icon-16.svg";
  import icon24 from "../../../../brand/svg/icon-24.svg";
  import CaptionButtons from "./CaptionButtons.svelte";
  import Icon from "./Icon.svelte";
  import { app } from "./app.svelte";
  import { appWindow } from "./native";
  import { openAddFromClipboard } from "./paste";

  let focused = $state(true);
  const settings = $derived(app.screen === "settings");
  const searchLabel = $derived(settings ? t("settings.search") : t("titlebar.search"));
  let search: HTMLInputElement | undefined = $state();

  onMount(() => {
    let off: (() => void) | undefined;
    void appWindow.onFocusChange((value) => (focused = value)).then((unlisten) => (off = unlisten));
    return () => off?.();
  });

  // Ctrl+F and "/" focus search (DESIGN-PRINCIPLES #6): the downloads', or
  // in Settings the settings'. Ctrl+K is the command palette (App.svelte).
  function onkeydown(event: KeyboardEvent) {
    if (app.adding || app.palette) return;
    const plain = !event.shiftKey && !event.altKey;
    const typing = (event.target as HTMLElement | null)?.closest("input, textarea, select");
    if (
      (event.ctrlKey && plain && event.code === "KeyF") ||
      (event.key === "/" && !event.ctrlKey && !typing)
    ) {
      event.preventDefault();
      search?.focus();
      search?.select();
    }
  }

  function onsearchkey(event: KeyboardEvent) {
    if (settings) {
      if (event.key === "Escape") {
        event.stopPropagation();
        if (app.settingsQuery) app.settingsQuery = "";
        else search?.blur();
      } else if (event.key === "ArrowDown" || event.key === "Enter") {
        // Into the first setting found.
        event.preventDefault();
        document.querySelector<HTMLElement>("[data-setting] :is(input, select, button)")?.focus();
      }
      return;
    }
    if (event.key === "Escape") {
      if (app.query) app.query = "";
      else search?.blur();
    } else if (event.key === "ArrowDown" || event.key === "Enter") {
      // Into the results.
      event.preventDefault();
      document.querySelector<HTMLElement>("[role='listbox']")?.focus();
    }
  }

  // The add sheet, prefilled with a link on the clipboard (like Ctrl+N).
  function addDownload() {
    void openAddFromClipboard();
  }
</script>

<svelte:window {onkeydown} />

<header class="titlebar" class:inactive={!focused} data-tauri-drag-region="deep">
  <div class="brand">
    <!-- The pixel-snapped app icon: the 16 px design at 1x and 2x, 24 px at 1.5x. -->
    <img
      src={icon16}
      srcset="{icon16} 1x, {icon24} 1.5x, {icon16} 2x"
      width="16"
      height="16"
      alt=""
      draggable="false"
    />
    <span class="name">{t("app.name")}</span>
  </div>

  <label class="search">
    <span class="search-icon"><Icon name="search" /></span>
    <input
      bind:this={search}
      bind:value={
        () => (settings ? app.settingsQuery : app.query),
        (v) => (settings ? (app.settingsQuery = v) : (app.query = v))
      }
      onkeydown={onsearchkey}
      id="titlebar-search"
      type="search"
      placeholder={searchLabel}
      aria-label={searchLabel}
      spellcheck="false"
      autocomplete="off"
    />
    <span class="search-key"><Keycap shortcut={t("shortcut.search")} /></span>
  </label>

  <div class="actions">
    <Button variant="primary" onclick={addDownload}>
      {#snippet icon()}<Icon name="plus" />{/snippet}
      {t("titlebar.add")}
    </Button>
  </div>

  <CaptionButtons />
</header>

<style>
  .titlebar {
    display: flex;
    align-items: center;
    height: var(--titlebar-height);
    background: var(--color-chrome);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: none;
    width: var(--sidebar-width);
    padding-left: var(--space-4);
  }
  .name {
    font-weight: var(--weight-title);
    letter-spacing: -0.005em;
  }

  .search {
    position: relative;
    display: flex;
    align-items: center;
    flex: 0 1 380px;
    min-width: 160px;
    height: var(--control-height);
    padding: 0 var(--space-1) 0 var(--space-2);
    gap: var(--space-2);
    border: 1px solid var(--color-hairline-strong);
    border-radius: var(--radius-control);
    background: var(--color-field);
    color: var(--color-text-subtle);
    cursor: text;
  }
  .search:focus-within {
    border-color: var(--color-focus);
    box-shadow: 0 0 0 1px var(--color-focus);
  }
  .search-icon {
    display: inline-flex;
  }
  .search input {
    flex: 1;
    min-width: 0;
    height: 100%;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--color-text);
    outline: none;
  }
  .search input::placeholder {
    color: var(--color-text-subtle);
  }
  .search input::-webkit-search-cancel-button {
    display: none;
  }
  .search-key {
    display: inline-flex;
  }
  .search:focus-within .search-key {
    visibility: hidden;
  }

  .actions {
    display: flex;
    align-items: center;
    margin-left: auto;
    padding: 0 var(--space-3);
  }

  .inactive .name,
  .inactive img {
    opacity: 0.6;
  }

  /* The icon rail: the brand column shrinks with the sidebar (APP-DESIGN #2). */
  @media (max-width: 899px) {
    .brand {
      width: var(--sidebar-rail-width);
      padding-left: 0;
      justify-content: center;
    }
    .name {
      display: none;
    }
  }
  @media (max-width: 719px) {
    .search-key {
      display: none;
    }
  }
</style>
