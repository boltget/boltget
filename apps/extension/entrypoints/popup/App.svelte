<!--
  The toolbar popup: the connection to the app, the capture switch, a pause,
  a notice the background left (a menu action that needs a word), and the
  site access prompt while it is missing.
-->
<script lang="ts">
  import AppMark from "@boltget/ui/components/AppMark.svelte";
  import Button from "@boltget/ui/components/Button.svelte";
  import Switch from "@boltget/ui/components/Switch.svelte";
  import { onMount } from "svelte";
  import { browser } from "wxt/browser";
  import { t } from "../../lib/i18n";
  import type { ConnectionStatus } from "../../lib/native";
  import {
    hasSiteAccess,
    localStore,
    reportColorScheme,
    requestSiteAccess,
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

  const paused = $derived(pausedUntil > now);
  const pausedLabel = $derived(
    new Date(pausedUntil).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" }),
  );

  onMount(() => {
    void reportColorScheme(matchMedia("(prefers-color-scheme: dark)").matches);
    void (async () => {
      const settings = await loadSettings(localStore);
      captureOn = settings.captureOn;
      pausedUntil = isPaused(settings, Date.now()) ? settings.pausedUntil : 0;
      notice = await takeNotice();
      access = await hasSiteAccess();
      status = await checkConnection();
    })();
    const tick = setInterval(() => (now = Date.now()), 15_000);
    return () => clearInterval(tick);
  });

  async function setCapture(on: boolean) {
    captureOn = on;
    await saveSettings(localStore, { captureOn: on });
  }

  async function pause() {
    pausedUntil = Date.now() + PAUSE_MS;
    now = Date.now();
    await saveSettings(localStore, { pausedUntil });
  }

  async function resume() {
    pausedUntil = 0;
    await saveSettings(localStore, { pausedUntil: 0 });
  }

  async function allow() {
    access = await requestSiteAccess();
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
