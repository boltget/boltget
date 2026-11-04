<!--
  The window: title bar, sidebar and the content area (APP-DESIGN #1). The
  chrome is one grey surface; the content sits on it as a white sheet with a
  hairline edge, like the website's hero card. The content is the list with
  its header and, on the right, the details pane; or, before there is a list,
  the engine's state or the empty state.
-->
<script lang="ts">
  import { t } from "@boltget/ui/messages";
  import { onMount } from "svelte";
  import { cubicOut } from "svelte/easing";
  import { fly } from "svelte/transition";
  import { app } from "./lib/app.svelte";
  import { clock } from "./lib/clock.svelte";
  import ConfirmDialog from "./lib/ConfirmDialog.svelte";
  import DetailsPane from "./lib/details/DetailsPane.svelte";
  import EmptyState from "./lib/EmptyState.svelte";
  import { downloads, errorText } from "./lib/engine";
  import { displayName } from "./lib/format";
  import DownloadList from "./lib/list/DownloadList.svelte";
  import ListHeader from "./lib/list/ListHeader.svelte";
  import Notice from "./lib/list/Notice.svelte";
  import AddSheet from "./lib/add/AddSheet.svelte";
  import { openAddFromClipboard, openAddFromText } from "./lib/paste";
  import Sidebar from "./lib/Sidebar.svelte";
  import TitleBar from "./lib/TitleBar.svelte";
  import Toasts from "./lib/Toasts.svelte";

  let count = $state(0);
  const status = $derived(downloads.status);
  const empty = $derived(downloads.ids.length === 0 && app.hidden.size === 0);
  const reduced =
    typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

  onMount(() => {
    clock.start();
    const stopErrors = app.reportErrors();
    // Completions and failures are announced politely; progress never is.
    const stopEvents = downloads.onEvent((event) => {
      const d = downloads.get(event.id);
      const name = d ? displayName(d.fileName, d.url) : "";
      if (event.kind === "completed") app.announce(t("announce.completed", { name }));
      if (event.kind === "stateChanged" && event.error) {
        app.announce(t("announce.failed", { name, reason: errorText(event.error) }));
      }
    });
    return () => {
      stopErrors();
      stopEvents();
    };
  });

  function editable(target: EventTarget | null): boolean {
    const el = target as HTMLElement | null;
    return !!el?.closest("input, textarea, [contenteditable='true']");
  }

  function onpaste(event: ClipboardEvent) {
    if (editable(event.target) || status.state !== "ready") return;
    const text = event.clipboardData?.getData("text") ?? "";
    if (openAddFromText(text)) event.preventDefault();
  }

  function ondragover(event: DragEvent) {
    const types = event.dataTransfer?.types ?? [];
    if (types.includes("text/uri-list") || types.includes("text/plain")) event.preventDefault();
  }

  function ondrop(event: DragEvent) {
    const data = event.dataTransfer;
    if (!data || status.state !== "ready") return;
    const text = data.getData("text/uri-list") || data.getData("text/plain");
    if (openAddFromText(text)) event.preventDefault();
  }

  function onkeydown(event: KeyboardEvent) {
    const ctrl = event.ctrlKey || event.metaKey;
    // The add sheet is modal: the window's shortcuts wait until it closes.
    if (app.adding) return;
    if (ctrl && !event.altKey && !event.shiftKey && event.code === "KeyN") {
      event.preventDefault();
      if (status.state === "ready") void openAddFromClipboard();
    } else if (ctrl && !event.altKey && event.code === "KeyI" && !event.shiftKey) {
      event.preventDefault();
      app.togglePane();
    } else if (ctrl && event.shiftKey && event.code === "KeyD") {
      event.preventDefault();
      app.setDensity(app.density === "compact" ? "comfortable" : "compact");
    } else if (ctrl && event.shiftKey && event.code === "KeyP") {
      event.preventDefault();
      app.pauseAll();
    } else if (ctrl && event.shiftKey && event.code === "KeyR") {
      event.preventDefault();
      app.resumeAll();
    }
  }
</script>

<svelte:window {onkeydown} {onpaste} {ondragover} {ondrop} />

<div class="window">
  <TitleBar />
  <div class="body">
    <Sidebar />
    <main class="content">
      {#if status.state === "starting"}
        <Notice title={t("engine.starting")} delayed />
      {:else if status.state === "locked"}
        <Notice
          icon="lock"
          tone="warning"
          title={t("engine.locked.title")}
          body={t("engine.locked")}
          detail={status.dir}
        />
      {:else if status.state === "failed"}
        <Notice
          icon="alert"
          tone="error"
          title={t("engine.failed.title")}
          body={errorText(status.error)}
          detail={status.error.message ?? undefined}
        />
      {:else if empty}
        <EmptyState />
      {:else}
        <div class="split">
          <section class="list" aria-label={t("list.label")}>
            <ListHeader {count} />
            <div class="list-body">
              <DownloadList oncount={(n) => (count = n)} />
            </div>
          </section>
          {#if app.paneOpen && app.selection.ids.size > 0}
            <div
              class="pane"
              in:fly={{ x: reduced ? 0 : 12, duration: reduced ? 0 : 220, easing: cubicOut }}
              out:fly={{ x: reduced ? 0 : 8, duration: reduced ? 0 : 150 }}
            >
              <DetailsPane />
            </div>
          {/if}
        </div>
      {/if}
      <Toasts />
    </main>
  </div>
</div>

<AddSheet />
<ConfirmDialog />
<div class="visually-hidden" aria-live="polite">{app.announcement}</div>

<style>
  .window {
    display: grid;
    grid-template-rows: var(--titlebar-height) minmax(0, 1fr);
    height: 100vh;
    background: var(--color-chrome);
  }
  .body {
    display: flex;
    min-height: 0;
  }
  .content {
    position: relative;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    background: var(--color-content);
    border-top: 1px solid var(--color-hairline);
    border-left: 1px solid var(--color-hairline);
    border-top-left-radius: var(--radius-floating);
  }
  .split {
    display: flex;
    height: 100%;
  }
  .list {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }
  .list-body {
    flex: 1;
    min-height: 0;
  }
  .pane {
    flex: none;
    width: var(--pane-width);
    border-left: 1px solid var(--color-hairline);
    background: var(--color-content);
  }

  /* Narrow windows: the pane floats over the list instead of squeezing it. */
  @media (max-width: 999px) {
    .pane {
      position: absolute;
      top: 0;
      right: 0;
      bottom: 0;
      z-index: 5;
      border-left: 0;
      box-shadow: var(--elevation-floating);
    }
  }
</style>
