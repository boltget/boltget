<!--
  The window: title bar, sidebar and the content area (APP-DESIGN #1). The
  chrome is one grey surface; the content sits on it as a white sheet with a
  hairline edge, like the website's hero card. The content is the list with
  its header and, on the right, the details pane; or, before there is a list,
  the engine's state or the empty state. Settings takes the place of the
  sidebar and the content (APP-DESIGN, "Settings as built"); the command
  palette (Ctrl+K) floats over either.
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
  import { hostOf } from "./lib/format";
  import {
    onClipboardOffer,
    onShellRequest,
    takeLaunchLinks,
    type ShellRequest,
  } from "./lib/native";
  import Palette from "./lib/palette/Palette.svelte";
  import Countdown from "./lib/queues/Countdown.svelte";
  import QueueSettings from "./lib/queues/QueueSettings.svelte";
  import StartLater from "./lib/queues/StartLater.svelte";
  import { openAddFromClipboard, openAddFromText } from "./lib/paste";
  import Settings from "./lib/settings/Settings.svelte";
  import { withCtrl } from "./lib/shortcuts";
  import Sidebar from "./lib/Sidebar.svelte";
  import { setThemeSetting } from "./lib/theme";
  import TitleBar from "./lib/TitleBar.svelte";
  import Toasts from "./lib/Toasts.svelte";

  let count = $state(0);
  /** Links from outside (a launch, the tray) waiting for the engine. */
  let incoming = $state<string | null>(null);
  /** How long a copied-link offer stays. */
  const OFFER_MS = 12_000;
  let clipboardWanted = $state(false);
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
    const stopShell = onShellRequest(onRequest);
    const stopOffers = onClipboardOffer(offer);
    void takeLaunchLinks().then((links) => {
      if (links) incoming = links;
    });
    return () => {
      stopErrors();
      stopEvents();
      void stopShell.then((stop) => stop());
      void stopOffers.then((stop) => stop());
    };
  });

  // The Theme setting, once the stored one is known (until then the page
  // keeps the theme it started with).
  $effect(() => {
    const theme = downloads.settings.theme;
    if (status.state === "ready") setThemeSetting(theme);
  });

  /**
   * Clipboard monitoring found links (APP-DESIGN #9): a quiet offer, never
   * a download. The add sheet opens only if the user takes it.
   */
  function offer(links: string) {
    const list = links.split("\n").filter(Boolean);
    const first = list[0];
    if (!first) return;
    const text =
      list.length === 1
        ? t("clipboard.offer", { name: displayName(null, first), host: hostOf(first) })
        : t("clipboard.offer.many", { count: list.length });
    const id = app.toast(
      text,
      {
        label: t("clipboard.offer.action"),
        run: () => {
          app.dismiss(id);
          app.closeSettings();
          openAddFromText(links);
        },
      },
      OFFER_MS,
    );
  }

  // The add sheet opens once the engine can probe.
  $effect(() => {
    if (status.state !== "ready") return;
    if (incoming !== null) {
      openAddFromText(incoming);
      incoming = null;
    }
    if (clipboardWanted) {
      clipboardWanted = false;
      void openAddFromClipboard();
    }
  });

  function onRequest(request: ShellRequest) {
    switch (request.kind) {
      case "add":
        incoming = request.links;
        break;
      case "addFromClipboard":
        clipboardWanted = true;
        break;
      case "select": {
        // Show it wherever it is: every download is in "All".
        app.reveal(request.id);
        const d = downloads.get(request.id);
        if (request.fix && d?.state === "needs_new_link") app.run("refresh", [request.id]);
        break;
      }
    }
  }

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

  // The window's shortcuts (src/lib/shortcuts.ts lists them for the user).
  function onkeydown(event: KeyboardEvent) {
    // Ctrl+K opens the palette, and closes it again.
    if (withCtrl(event) && event.code === "KeyK") {
      event.preventDefault();
      if (!app.adding) app.palette = !app.palette;
      return;
    }
    // The add sheet, the palette and the dialogs are modal: the window's
    // shortcuts wait.
    if (app.adding || app.palette || app.queueEditing || app.startingLater) return;
    if (downloads.countdown) return;
    if (withCtrl(event) && event.code === "KeyN") {
      event.preventDefault();
      if (status.state === "ready") void openAddFromClipboard();
    } else if (withCtrl(event) && (event.code === "Comma" || event.key === ",")) {
      event.preventDefault();
      if (app.screen === "settings") app.closeSettings();
      else app.openSettings();
    } else if (withCtrl(event) && event.code === "KeyI") {
      event.preventDefault();
      app.closeSettings();
      app.togglePane();
    } else if (withCtrl(event, true) && event.code === "KeyD") {
      event.preventDefault();
      app.setDensity(app.density === "compact" ? "comfortable" : "compact");
    } else if (withCtrl(event, true) && event.code === "KeyP") {
      event.preventDefault();
      app.pauseAll();
    } else if (withCtrl(event, true) && event.code === "KeyR") {
      event.preventDefault();
      app.resumeAll();
    }
  }
</script>

<svelte:window {onkeydown} {onpaste} {ondragover} {ondrop} />

<div class="window">
  <TitleBar />
  <div class="body">
    {#if app.screen === "settings"}
      <Settings />
    {:else}
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
    {/if}
  </div>
</div>

<AddSheet />
<Palette />
<ConfirmDialog />
<QueueSettings />
<StartLater />
<Countdown />
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
