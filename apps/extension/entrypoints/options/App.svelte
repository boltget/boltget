<!--
  The options page: onboarding (opened with #welcome after install), site
  access with an honest explanation, the connection with Test connection, the
  capture rules, the site lists and the modifier keys. Changes apply as they
  are made, as in the app's settings: there is no Save.
-->
<script lang="ts">
  import type { CaptureRules } from "@boltget/native-messaging";
  import AppMark from "@boltget/ui/components/AppMark.svelte";
  import Button from "@boltget/ui/components/Button.svelte";
  import Switch from "@boltget/ui/components/Switch.svelte";
  import TextField from "@boltget/ui/components/TextField.svelte";
  import { onMount } from "svelte";
  import { t } from "../../lib/i18n";
  import type { ConnectionStatus } from "../../lib/native";
  import {
    FLAVOR,
    hasSiteAccess,
    localStore,
    removeSiteAccess,
    reportColorScheme,
    requestSiteAccess,
  } from "../../lib/platform";
  import { normaliseExtension, normaliseSite, splitList } from "../../lib/rules";
  import { editRules, loadSettings, resetRules, saveSettings } from "../../lib/settings";
  import { checkConnection } from "../../lib/ui/connection";
  import StatusCard from "../../lib/ui/StatusCard.svelte";

  const MIB = 1024 * 1024;

  const welcome = location.hash === "#welcome";
  let status = $state<ConnectionStatus | null>(null);
  let testing = $state(false);
  let access = $state(false);
  let loaded = $state(false);
  let edited = $state(false);
  let saved = $state(false);

  let enabled = $state(true);
  let minSize = $state("1");
  let extensions = $state("");
  let neverSites = $state("");
  let alwaysSites = $state("");
  let privateWindows = $state(false);
  let modifierKeys = $state(true);

  function show(rules: CaptureRules) {
    enabled = rules.enabled;
    minSize = String(Math.round((rules.min_size / MIB) * 100) / 100);
    extensions = rules.extensions.join(" ");
    neverSites = rules.never_sites.join("\n");
    alwaysSites = rules.always_sites.join("\n");
    privateWindows = rules.private_windows;
  }

  async function load() {
    const settings = await loadSettings(localStore);
    show(settings.rules);
    edited = settings.rulesEdited;
    modifierKeys = settings.modifierKeys;
    loaded = true;
  }

  onMount(() => {
    void reportColorScheme(matchMedia("(prefers-color-scheme: dark)").matches);
    void (async () => {
      await load();
      access = await hasSiteAccess();
      status = await checkConnection();
      // hello may have brought the app's rules.
      await load();
    })();
  });

  let savedTimer: ReturnType<typeof setTimeout> | undefined;
  function flashSaved() {
    saved = true;
    clearTimeout(savedTimer);
    savedTimer = setTimeout(() => (saved = false), 1500);
  }

  async function saveRules() {
    const size = Number.parseFloat(minSize.replace(",", "."));
    const rules: CaptureRules = {
      enabled,
      min_size: Number.isFinite(size) && size >= 0 ? Math.round(size * MIB) : MIB,
      extensions: splitList(extensions)
        .map(normaliseExtension)
        .filter((e) => e !== null),
      never_sites: splitList(neverSites)
        .map(normaliseSite)
        .filter((s) => s !== null),
      always_sites: splitList(alwaysSites)
        .map(normaliseSite)
        .filter((s) => s !== null),
      private_windows: privateWindows,
    };
    await editRules(localStore, rules);
    edited = true;
    await load();
    flashSaved();
  }

  async function reset() {
    show(await resetRules(localStore));
    edited = false;
    flashSaved();
  }

  async function test() {
    testing = true;
    status = null;
    status = await checkConnection();
    testing = false;
    await load();
  }

  async function allow() {
    access = await requestSiteAccess();
  }

  async function remove() {
    await removeSiteAccess();
    access = await hasSiteAccess();
  }

  async function setModifierKeys(on: boolean) {
    modifierKeys = on;
    await saveSettings(localStore, { modifierKeys: on });
    flashSaved();
  }
</script>

<main>
  <header>
    <AppMark size={28} />
    <h1>{t("options.title")}</h1>
    {#if FLAVOR === "development"}<span class="tag">{t("options.development")}</span>{/if}
    <span class="saved" class:visible={saved} aria-live="polite">
      {saved ? t("options.saved") : ""}
    </span>
  </header>

  {#if welcome}
    <section class="welcome">
      <h2>{t("welcome.title")}</h2>
      <p>{t("welcome.body")}</p>
    </section>
  {/if}

  <section class:highlight={welcome && !access}>
    <h2>{t("access.title")}</h2>
    <p class="body">{t("access.body")}</p>
    <p class="hint" data-testid="access-state">
      {access ? t("access.granted") : t("access.missing")}
    </p>
    <div class="actions">
      {#if access}
        <Button onclick={remove}>{t("access.remove")}</Button>
      {:else}
        <Button variant="primary" onclick={allow}>{t("access.allow")}</Button>
      {/if}
    </div>
  </section>

  <section>
    <h2>{t("connection.title")}</h2>
    <StatusCard {status} />
    <div class="actions">
      <Button onclick={test} disabled={testing}>
        {testing ? t("connection.testing") : t("connection.test")}
      </Button>
    </div>
  </section>

  {#if loaded}
    <section>
      <h2>{t("rules.title")}</h2>
      <p class="hint">{edited ? t("rules.edited") : t("rules.from_app")}</p>

      <div class="row">
        <span id="enabled-label" class="label">{t("rules.enabled")}</span>
        <Switch
          bind:checked={enabled}
          aria-labelledby="enabled-label"
          onchange={() => saveRules()}
        />
      </div>

      <div class="field">
        <label for="min-size" class="label">{t("rules.min_size")}</label>
        <div class="narrow">
          <TextField
            id="min-size"
            inputmode="decimal"
            bind:value={minSize}
            unit={t("rules.min_size_unit")}
            onchange={() => saveRules()}
            aria-describedby="min-size-hint"
          />
        </div>
        <p id="min-size-hint" class="hint">{t("rules.min_size_hint")}</p>
      </div>

      <div class="field">
        <label for="extensions" class="label">{t("rules.extensions")}</label>
        <textarea
          id="extensions"
          rows="3"
          spellcheck="false"
          bind:value={extensions}
          onchange={() => saveRules()}
          aria-describedby="extensions-hint"></textarea>
        <p id="extensions-hint" class="hint">{t("rules.extensions_hint")}</p>
      </div>

      <div class="row">
        <div>
          <span id="private-label" class="label">{t("rules.private")}</span>
          <p class="hint">{t("rules.private_hint")}</p>
        </div>
        <Switch
          bind:checked={privateWindows}
          aria-labelledby="private-label"
          onchange={() => saveRules()}
        />
      </div>

      {#if edited}
        <div class="actions">
          <Button onclick={reset}>{t("rules.reset")}</Button>
        </div>
      {/if}
    </section>

    <section>
      <h2>{t("sites.title")}</h2>
      <div class="field">
        <label for="never" class="label">{t("sites.never")}</label>
        <textarea
          id="never"
          rows="4"
          spellcheck="false"
          bind:value={neverSites}
          onchange={() => saveRules()}
          aria-describedby="never-hint"></textarea>
        <p id="never-hint" class="hint">{t("sites.never_hint")}</p>
      </div>
      <div class="field">
        <label for="always" class="label">{t("sites.always")}</label>
        <textarea
          id="always"
          rows="4"
          spellcheck="false"
          bind:value={alwaysSites}
          onchange={() => saveRules()}
          aria-describedby="always-hint"></textarea>
        <p id="always-hint" class="hint">{t("sites.always_hint")}</p>
      </div>
    </section>

    <section>
      <h2>{t("keys.title")}</h2>
      <div class="row">
        <div>
          <span id="keys-label" class="label">{t("keys.modifiers")}</span>
          <p class="hint">{t("keys.modifiers_hint")}</p>
        </div>
        <Switch
          checked={modifierKeys}
          aria-labelledby="keys-label"
          onchange={(e) => setModifierKeys(e.currentTarget.checked)}
        />
      </div>
    </section>
  {/if}
</main>

<style>
  :global(html),
  :global(body) {
    height: auto;
    min-height: 100%;
    overflow: auto;
  }
  main {
    max-width: 640px;
    margin: 0 auto;
    padding: var(--space-8) var(--space-6) var(--space-12);
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
  }
  header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-20);
    line-height: var(--leading-20);
    font-weight: var(--weight-title);
    letter-spacing: var(--tracking-20);
  }
  .tag {
    padding: 0 var(--space-2);
    border: 1px solid var(--color-hairline-strong);
    border-radius: var(--radius-control);
    color: var(--color-text-muted);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }
  .saved {
    margin-left: auto;
    color: var(--color-text-muted);
    font-size: var(--text-12);
    opacity: 0;
    transition: opacity var(--duration-small) var(--ease-exit);
  }
  .saved.visible {
    opacity: 1;
  }
  section {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-5);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-floating);
    background: var(--color-raised);
  }
  section.highlight {
    border-color: var(--color-accent);
  }
  .welcome {
    background: var(--color-accent-tint);
    border-color: transparent;
  }
  h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-15);
    line-height: var(--leading-15);
    font-weight: var(--weight-title);
    letter-spacing: var(--tracking-15);
  }
  p {
    margin: 0;
  }
  .body {
    max-width: 60ch;
    user-select: text;
  }
  .label {
    font-weight: var(--weight-medium);
  }
  .hint {
    color: var(--color-text-muted);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }
  .row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
  }
  .row .hint {
    margin-top: 2px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .narrow {
    width: 160px;
  }
  .actions {
    display: flex;
    gap: var(--space-2);
  }
  textarea {
    width: 100%;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--color-hairline-strong);
    border-radius: var(--radius-control);
    background: var(--color-field);
    color: var(--color-text);
    font-family: var(--font-mono);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    resize: vertical;
    user-select: text;
  }
  textarea:focus-visible {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: 0 0 0 1px var(--color-accent);
  }
</style>
