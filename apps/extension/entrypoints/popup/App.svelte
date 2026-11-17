<!--
  The toolbar popup: the connection to the app, the capture switch, a pause,
  a notice the background left (a menu action that needs a word), and the
  site access prompt while it is missing.

  On a page where media may be offered (lib/policy.ts: never on the sites the
  store policy excludes), it also offers "Download video from this page" and
  lists the media the page loaded (lib/media.ts); either opens the app's
  media sheet, where the user picks the quality.
-->
<script lang="ts">
  import AppMark from "@boltget/ui/components/AppMark.svelte";
  import Button from "@boltget/ui/components/Button.svelte";
  import Switch from "@boltget/ui/components/Switch.svelte";
  import { onMount } from "svelte";
  import { browser } from "wxt/browser";
  import { t } from "../../lib/i18n";
  import { hintLine, type MediaSendResult, type MediaState, type PageScan } from "../../lib/media";
  import type { ConnectionStatus } from "../../lib/native";
  import {
    hasSiteAccess,
    localStore,
    popupTab,
    reportColorScheme,
    requestSiteAccess,
    scanTab,
    sendTabMedia,
    tabMediaState,
    takeNotice,
    type NoticeKey,
  } from "../../lib/platform";
  import { isPaused, loadSettings, PAUSE_MS, saveSettings } from "../../lib/settings";
  import { checkConnection } from "../../lib/ui/connection";
  import StatusCard from "../../lib/ui/StatusCard.svelte";

  let status = $state<ConnectionStatus | null>(null);
  let captureOn = $state(true);
  let pausedUntil = $state(0);
  let access = $state(true);
  let notice = $state<NoticeKey | null>(null);
  let now = $state(Date.now());

  // The page's media: shown only where the background allows it.
  let mediaTab: number | null = null;
  let media = $state.raw<MediaState | null>(null);
  let scan = $state.raw<PageScan | null>(null);
  let sending = $state<"idle" | "sending" | MediaSendResult>("idle");
  const found = $derived(
    !!media && (media.hints.length > 0 || (scan !== null && (scan.elements > 0 || scan.declared))),
  );
  const sendingText = $derived(
    sending === "sending"
      ? t("media.sending")
      : sending === "sent"
        ? t("media.sent")
        : sending === "app_too_old"
          ? t("media.app_too_old")
          : sending === "failed" || sending === "excluded"
            ? t("media.failed")
            : "",
  );

  const paused = $derived(pausedUntil > now);
  const pausedLabel = $derived(
    new Date(pausedUntil).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" }),
  );

  // A click can land before the stored settings arrive; what the user set
  // then must not be overwritten by the older stored value.
  let touchedCapture = false;
  let touchedPause = false;

  onMount(() => {
    void reportColorScheme(matchMedia("(prefers-color-scheme: dark)").matches);
    void (async () => {
      const settings = await loadSettings(localStore);
      if (!touchedCapture) captureOn = settings.captureOn;
      if (!touchedPause) pausedUntil = isPaused(settings, Date.now()) ? settings.pausedUntil : 0;
      notice = await takeNotice();
      access = await hasSiteAccess();
      void loadMedia();
      status = await checkConnection();
    })();
    const tick = setInterval(() => (now = Date.now()), 15_000);
    return () => clearInterval(tick);
  });

  async function setCapture(on: boolean) {
    touchedCapture = true;
    captureOn = on;
    await saveSettings(localStore, { captureOn: on });
  }

  async function pause() {
    touchedPause = true;
    pausedUntil = Date.now() + PAUSE_MS;
    now = Date.now();
    await saveSettings(localStore, { pausedUntil });
  }

  async function resume() {
    touchedPause = true;
    pausedUntil = 0;
    await saveSettings(localStore, { pausedUntil: 0 });
  }

  async function allow() {
    access = await requestSiteAccess();
  }

  async function loadMedia() {
    const tabId = await popupTab();
    if (tabId === null) return;
    const state = await tabMediaState(tabId);
    if (!state?.allowed) return;
    mediaTab = tabId;
    media = state;
    // Only where media is offered does the popup look into the page.
    scan = await scanTab(tabId);
  }

  async function sendMedia(hint: string | null) {
    if (mediaTab === null || sending === "sending") return;
    sending = "sending";
    sending = await sendTabMedia(mediaTab, hint, hint === null ? (scan?.sources ?? []) : []);
    if (sending === "sent") setTimeout(() => window.close(), 1200);
  }

  function openSettings() {
    void browser.runtime.openOptionsPage();
    window.close();
  }
</script>

<main>
  <header>
    <AppMark size={20} />
    <h1>{t("app.name")}</h1>
  </header>

  {#if notice}
    <section class="notice" role="alert">
      <p>{t(notice)}</p>
      <Button variant="ghost" onclick={() => (notice = null)}>{t("notice.dismiss")}</Button>
    </section>
  {/if}

  <section>
    <StatusCard {status} />
  </section>

  <section class="row">
    <div class="words">
      <p id="capture-label" class="label">{t("popup.capture")}</p>
      <p class="hint">
        {#if !captureOn}
          {t("popup.capture_off")}
        {:else if paused}
          {t("popup.paused", { time: pausedLabel })}
        {:else}
          {t("popup.capture_on")}
        {/if}
      </p>
    </div>
    <Switch
      checked={captureOn}
      aria-labelledby="capture-label"
      onchange={(e) => setCapture(e.currentTarget.checked)}
    />
  </section>

  {#if captureOn}
    <section class="actions">
      {#if paused}
        <Button onclick={resume}>{t("popup.resume")}</Button>
      {:else}
        <Button onclick={pause}>{t("popup.pause")}</Button>
      {/if}
    </section>
  {/if}

  {#if media}
    <section class="media" aria-labelledby="media-label">
      <div class="words">
        <p id="media-label" class="label">{t("media.title")}</p>
        <p class="hint">{found ? t("media.found") : t("media.maybe")}</p>
      </div>
      <Button
        variant={found ? "primary" : "secondary"}
        disabled={sending === "sending"}
        onclick={() => sendMedia(null)}>{t("media.download")}</Button
      >
      {#if media.hints.length}
        <p id="media-seen" class="hint">{t("media.seen")}</p>
        <ul class="hints" aria-labelledby="media-seen">
          {#each media.hints as hint (hint.url)}
            {@const line = hintLine(hint)}
            <li>
              <span class="hint-text">
                <span class="hint-kind">{t(line.kind)}</span>
                <span class="hint-details">{line.details.join(" · ")}</span>
              </span>
              <Button
                variant="ghost"
                disabled={sending === "sending"}
                aria-label={t("media.use_label", {
                  kind: t(line.kind).toLowerCase(),
                  host: line.host,
                })}
                onclick={() => sendMedia(hint.url)}>{t("media.use")}</Button
              >
            </li>
          {/each}
        </ul>
      {/if}
      <p class="hint" class:error={sending === "failed" || sending === "app_too_old"} role="status">
        {sendingText}
      </p>
    </section>
  {/if}

  {#if !access}
    <section class="access">
      <p class="hint">{t("popup.access_hint")}</p>
      <Button variant="primary" onclick={allow}>{t("popup.access")}</Button>
    </section>
  {/if}

  <footer>
    <Button variant="ghost" onclick={openSettings}>{t("popup.settings")}</Button>
  </footer>
</main>

<style>
  :global(html),
  :global(body) {
    height: auto;
  }
  main {
    width: 320px;
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }
  header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-15);
    line-height: var(--leading-15);
    font-weight: var(--weight-title);
    letter-spacing: var(--tracking-15);
  }
  section {
    margin: 0;
  }
  p {
    margin: 0;
  }
  .row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    padding-top: var(--space-4);
    border-top: 1px solid var(--color-hairline);
  }
  .label {
    font-weight: var(--weight-medium);
  }
  .hint {
    margin-top: 2px;
    color: var(--color-text-muted);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }
  .actions {
    display: flex;
    gap: var(--space-2);
  }
  .notice {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-floating);
    background: var(--color-accent-tint);
  }
  .media {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    padding-top: var(--space-4);
    border-top: 1px solid var(--color-hairline);
  }
  .media .hint:empty {
    display: none;
  }
  .media .hint.error {
    color: var(--color-error);
  }
  .hints {
    align-self: stretch;
    margin: 0;
    padding: 0;
    list-style: none;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-floating);
    overflow: hidden;
  }
  .hints li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-1) var(--space-1) var(--space-3);
  }
  .hints li + li {
    border-top: 1px solid var(--color-hairline);
  }
  .hint-text {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }
  .hint-kind {
    font-size: var(--text-12);
    line-height: var(--leading-12);
    font-weight: var(--weight-medium);
  }
  .hint-details {
    color: var(--color-text-muted);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .access {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--space-2);
    padding-top: var(--space-4);
    border-top: 1px solid var(--color-hairline);
  }
  footer {
    display: flex;
    justify-content: flex-end;
    margin: calc(var(--space-2) * -1) calc(var(--space-2) * -1) calc(var(--space-2) * -1) 0;
  }
</style>
