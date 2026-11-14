<!--
  Settings (APP-DESIGN #17, "Settings as built"): a screen of the main
  window, not a window of its own. The sections are on the left, where the
  sidebar is; the title bar's search finds settings across them. Each
  setting is a row with a label, one sentence and its control, and applies
  when it changes: switches and choices at once, typed values on Enter or
  when the field is left, after a check that says what is wrong under the
  field. Esc (outside a field) goes back to the downloads.
-->
<script lang="ts">
  import Button from "@boltget/ui/components/Button.svelte";
  import IconButton from "@boltget/ui/components/IconButton.svelte";
  import Keycap from "@boltget/ui/components/Keycap.svelte";
  import Select from "@boltget/ui/components/Select.svelte";
  import Switch from "@boltget/ui/components/Switch.svelte";
  import TextField from "@boltget/ui/components/TextField.svelte";
  import { t } from "@boltget/ui/messages";
  import { onMount, tick, untrack } from "svelte";
  import { app } from "../app.svelte";
  import type {
    AppSettings,
    CategoryId,
    ProxyMode,
    SettingsPatch,
    SystemProxyInfo,
    ThemeSetting,
  } from "../bindings";
  import { downloads, errorText } from "../engine";
  import { splitFolder } from "../format";
  import Icon, { type IconName } from "../Icon.svelte";
  import { CATEGORY_LABELS } from "../list/category";
  import { appInfo, type AppInfo } from "../native";
  import { SHORTCUTS } from "../shortcuts";
  import { setThemeSetting } from "../theme";
  import {
    formatSpeedLimit,
    parseConnections,
    parseExtensions,
    parseProxyUrl,
    parseSpeedLimit,
  } from "./model";
  import { ROWS, SECTIONS, searchRows, type RowId, type SectionId } from "./registry";
  import SettingRow from "./SettingRow.svelte";
  import Toasts from "../Toasts.svelte";

  const s = $derived(downloads.settings);
  const searching = $derived(app.settingsQuery.trim().length > 0);
  const found = $derived(searchRows(app.settingsQuery));
  /** The rows on show: the section's, or what the search found. */
  const shown = $derived(
    new Set<RowId>(
      searching
        ? found.map((r) => r.id)
        : ROWS.filter((r) => r.section === app.settingsSection).map((r) => r.id),
    ),
  );
  const sections = $derived(
    SECTIONS.filter((sec) => ROWS.some((r) => r.section === sec.id && shown.has(r.id))),
  );

  let content: HTMLElement | undefined = $state();
  let info = $state<AppInfo | null>(null);
  let system = $state<SystemProxyInfo | null>(null);
  let highlighted = $state<RowId | null>(null);

  // Problems by field, and drafts of typed values.
  let problems = $state<Record<string, string | null>>({});
  let editing = $state<string | null>(null);
  let connections = $state("");
  let limit = $state("");
  let proxyUrl = $state("");
  let proxyBypass = $state("");
  let extensions = $state<Partial<Record<CategoryId, string>>>({});
  /** "Use this proxy" chosen, waiting for a valid address before it applies. */
  let manualPending = $state(false);
  const proxyMode = $derived<ProxyMode>(manualPending ? "manual" : s.proxy.mode);

  const categoryName = (id: CategoryId) => t(CATEGORY_LABELS[id]);
  const CATEGORY_ICONS: Record<CategoryId, IconName> = {
    video: "video",
    music: "music",
    archives: "archives",
    programs: "programs",
    documents: "documents",
    other: "other",
  };

  // Drafts follow the settings, except the field being typed in.
  $effect(() => {
    const current = s;
    untrack(() => syncDrafts(current));
  });

  function syncDrafts(current: AppSettings) {
    if (editing !== "connections") connections = String(current.maxConnections);
    if (editing !== "limit") limit = formatSpeedLimit(current.speedLimit);
    if (editing !== "proxyUrl" && !manualPending) proxyUrl = current.proxy.url;
    if (editing !== "proxyBypass") proxyBypass = current.proxy.bypass;
    const next = { ...extensions };
    for (const c of current.categories) {
      if (editing !== `ext.${c.id}`) next[c.id] = c.extensions.join(" ");
    }
    extensions = next;
  }

  onMount(() => {
    void appInfo().then((i) => (info = i));
    void refreshSystemProxy();
    // The list that had focus is gone: focus the current section, so the
    // keyboard and a screen reader carry on in Settings (unless the palette
    // or a link put focus on a setting).
    void tick().then(() => {
      if (document.activeElement === document.body || !document.activeElement) {
        document.getElementById(`settings-nav-${app.settingsSection}`)?.focus();
      }
    });
  });

  async function refreshSystemProxy() {
    try {
      system = await downloads.systemProxy();
    } catch {
      system = null;
    }
  }

  // The palette (or a link) asked for a setting: show it and focus it.
  $effect(() => {
    const target = app.settingsTarget;
    if (!target) return;
    untrack(() => void reveal(target.row));
  });

  async function reveal(row: RowId) {
    await tick();
    const el = content?.querySelector<HTMLElement>(`[data-setting="${row}"]`);
    if (!el) return;
    el.scrollIntoView({ block: "center" });
    highlighted = row;
    setTimeout(() => {
      if (highlighted === row) highlighted = null;
    }, 1200);
    el.querySelector<HTMLElement>("input, select, button")?.focus({ preventScroll: true });
  }

  // Saving -------------------------------------------------------------------

  function problem(field: string, text: string | null) {
    problems = { ...problems, [field]: text };
  }

  /** Sends a change; a refusal shows under `field`. */
  async function save(field: string, patch: SettingsPatch): Promise<boolean> {
    const result = await downloads.updateSettings(patch);
    if (result.status === "error") {
      problem(field, errorText(result.error));
      return false;
    }
    problem(field, null);
    return true;
  }

  function onswitch(field: string, patch: (on: boolean) => SettingsPatch) {
    return (event: Event) => {
      const input = event.currentTarget as HTMLInputElement;
      const on = input.checked;
      void save(field, patch(on)).then((saved) => {
        if (!saved) input.checked = !on;
      });
    };
  }

  function setTheme(theme: ThemeSetting) {
    setThemeSetting(theme);
    void save("theme", { theme });
  }

  /** Typed fields apply on Enter, or when the field is left. */
  function commitOn(event: KeyboardEvent, commit: () => void) {
    if (event.key === "Enter") {
      event.preventDefault();
      commit();
    } else if (event.key === "Escape") {
      // Puts the stored value back, rather than leaving Settings.
      event.stopPropagation();
      editing = null;
      problems = {};
      syncDrafts(s);
      (event.currentTarget as HTMLElement).blur();
    }
  }

  function focusField(field: string) {
    return () => (editing = field);
  }

  function commitConnections() {
    editing = null;
    const parsed = parseConnections(connections);
    if (!parsed.ok) return problem("connections", parsed.problem);
    if (parsed.value === s.maxConnections) return problem("connections", null);
    void save("connections", { maxConnections: parsed.value });
  }

  async function commitLimit() {
    editing = null;
    const parsed = parseSpeedLimit(limit);
    if (!parsed.ok) return problem("limit", parsed.problem);
    problem("limit", null);
    if (parsed.value === s.speedLimit) return;
    const result = await downloads.setSpeedLimit({ kind: "global" }, parsed.value);
    if (result.status === "error") problem("limit", errorText(result.error));
  }

  function commitExtensions(id: CategoryId) {
    editing = null;
    const field = `ext.${id}`;
    const parsed = parseExtensions(extensions[id] ?? "", id, s.categories, categoryName);
    if (!parsed.ok) return problem(field, parsed.problem);
    const current = s.categories.find((c) => c.id === id)?.extensions.join(" ");
    if (parsed.value.join(" ") === current) {
      extensions = { ...extensions, [id]: current };
      return problem(field, null);
    }
    void save(field, { category: { id, extensions: parsed.value.join(" "), folder: null } });
  }

  async function chooseFolder(
    field: string,
    start: string | null,
    apply: (path: string) => SettingsPatch,
  ) {
    const result = await downloads.chooseFolder(start);
    if (result.status === "ok" && result.data) void save(field, apply(result.data));
  }

  function chooseProxyMode(mode: ProxyMode) {
    problem("proxy", null);
    if (mode === "manual") {
      const parsed = parseProxyUrl(proxyUrl);
      if (parsed.ok) {
        manualPending = false;
        void save("proxy", { proxy: { mode, url: parsed.value, bypass: proxyBypass.trim() } });
      } else {
        manualPending = true;
        void tick().then(() => document.getElementById("proxy-url")?.focus());
      }
      return;
    }
    manualPending = false;
    if (mode === "system") void refreshSystemProxy();
    void save("proxy", { proxy: { mode, url: s.proxy.url, bypass: s.proxy.bypass } });
  }

  function commitProxy() {
    editing = null;
    const parsed = parseProxyUrl(proxyUrl);
    if (!parsed.ok) return problem("proxy", parsed.problem);
    const bypass = proxyBypass.trim();
    if (!manualPending && parsed.value === s.proxy.url && bypass === s.proxy.bypass) {
      return problem("proxy", null);
    }
    void save("proxy", { proxy: { mode: "manual", url: parsed.value, bypass } }).then((saved) => {
      if (saved) manualPending = false;
    });
  }

  // Keyboard -----------------------------------------------------------------

  function onkeydown(event: KeyboardEvent) {
    if (event.key !== "Escape" || app.palette || app.adding) return;
    // Esc in a text field puts its stored value back; elsewhere (a switch,
    // a section, a button) it returns to the downloads.
    const el = event.target as HTMLElement | null;
    if (el?.closest("textarea, input:not([type='checkbox'], [type='radio'], [type='button'])"))
      return;
    event.preventDefault();
    app.closeSettings();
  }

  function onnavkey(event: KeyboardEvent) {
    // Up and down move between sections, like the sidebar's views.
    const order = SECTIONS.map((sec) => sec.id);
    const at = order.indexOf(app.settingsSection);
    const next = event.key === "ArrowDown" ? at + 1 : event.key === "ArrowUp" ? at - 1 : Number.NaN;
    const id = order[next];
    if (!id) return;
    event.preventDefault();
    choose(id);
    document.getElementById(`settings-nav-${id}`)?.focus();
  }

  function choose(id: SectionId) {
    app.settingsQuery = "";
    app.settingsSection = id;
    content?.scrollTo({ top: 0 });
  }

  const describedBy = (id: string, field?: string) =>
    [`setting-${id}-description`, field && problems[field] ? `setting-${id}-problem` : ""]
      .filter(Boolean)
      .join(" ");
</script>

<svelte:window {onkeydown} />

{#snippet folderLabel(path: string | null, fallback: string)}
  {#if path}
    {@const parts = splitFolder(path)}
    <span class="path" title={path}
      ><span class="parent"><span dir="ltr">{parts.parent}</span></span><span class="leaf"
        >{parts.leaf}</span
      ></span
    >
  {:else}
    <span class="path muted">{fallback}</span>
  {/if}
{/snippet}

<div class="settings">
  <nav class="nav" aria-label={t("settings.sections")}>
    <button type="button" class="item back" onclick={() => app.closeSettings()}>
      <Icon name="back" />
      <span class="item-label">{t("settings.back")}</span>
    </button>
    <h2 class="heading">{t("settings.title")}</h2>
    <ul>
      {#each SECTIONS as section (section.id)}
        <li>
          <button
            type="button"
            id="settings-nav-{section.id}"
            class="item"
            aria-current={!searching && app.settingsSection === section.id ? "page" : undefined}
            title={t(section.label)}
            onclick={() => choose(section.id)}
            onkeydown={onnavkey}
          >
            <Icon name={section.icon} />
            <span class="item-label">{t(section.label)}</span>
          </button>
        </li>
      {/each}
    </ul>
  </nav>

  <main class="frame" aria-label={t("settings.title")}>
    <div class="content" bind:this={content}>
      <div class="column">
        {#if searching && found.length === 0}
          <p class="none">{t("settings.search.none", { query: app.settingsQuery.trim() })}</p>
        {/if}
        {#each sections as section (section.id)}
          <div class="section" aria-labelledby="section-{section.id}">
            <h2 class="title" id="section-{section.id}">{t(section.label)}</h2>

            {#if shown.has("theme")}
              <SettingRow
                id="theme"
                label={t("settings.theme")}
                description={t("settings.theme.description")}
                target={highlighted === "theme"}
                problem={problems.theme}
              >
                {#snippet control()}
                  <Select
                    value={s.theme}
                    aria-labelledby="setting-theme"
                    aria-describedby={describedBy("theme", "theme")}
                    options={[
                      { value: "system", label: t("settings.theme.system") },
                      { value: "light", label: t("settings.theme.light") },
                      { value: "dark", label: t("settings.theme.dark") },
                    ]}
                    onchange={(e) => setTheme(e.currentTarget.value as ThemeSetting)}
                  />
                {/snippet}
              </SettingRow>
            {/if}

            {#if shown.has("autostart")}
              <SettingRow
                id="autostart"
                label={t("settings.autostart")}
                description={t("settings.autostart.description")}
                target={highlighted === "autostart"}
                problem={problems.autostart}
              >
                {#snippet control()}
                  <Switch
                    checked={s.startWithWindows}
                    aria-labelledby="setting-autostart"
                    aria-describedby={describedBy("autostart", "autostart")}
                    onchange={onswitch("autostart", (startWithWindows) => ({ startWithWindows }))}
                  />
                {/snippet}
              </SettingRow>
            {/if}

            {#if shown.has("tray")}
              <SettingRow
                id="tray"
                label={t("settings.tray")}
                description={t("settings.tray.description")}
                target={highlighted === "tray"}
                problem={problems.tray}
              >
                {#snippet control()}
                  <Switch
                    checked={s.closeToTray}
                    aria-labelledby="setting-tray"
                    aria-describedby={describedBy("tray", "tray")}
                    onchange={onswitch("tray", (closeToTray) => ({ closeToTray }))}
                  />
                {/snippet}
              </SettingRow>
            {/if}

            {#if shown.has("notifications")}
              <SettingRow
                id="notifications"
                label={t("settings.notifications")}
                description={t("settings.notifications.description")}
                target={highlighted === "notifications"}
                problem={problems.notifications}
              >
                {#snippet control()}
                  <Switch
                    checked={s.notifications}
                    aria-labelledby="setting-notifications"
                    aria-describedby={describedBy("notifications", "notifications")}
                    onchange={onswitch("notifications", (notifications) => ({ notifications }))}
                  />
                {/snippet}
              </SettingRow>
            {/if}

            {#if shown.has("clipboard")}
              <SettingRow
                id="clipboard"
                label={t("settings.clipboard")}
                description={t("settings.clipboard.description")}
                target={highlighted === "clipboard"}
                problem={problems.clipboard}
              >
                {#snippet control()}
                  <Switch
                    checked={s.clipboard}
                    aria-labelledby="setting-clipboard"
                    aria-describedby={describedBy("clipboard", "clipboard")}
                    onchange={onswitch("clipboard", (clipboard) => ({ clipboard }))}
                  />
                {/snippet}
              </SettingRow>
            {/if}

            {#if shown.has("folder")}
              <SettingRow
                id="folder"
                label={t("settings.folder")}
                description={t("settings.folder.description")}
                target={highlighted === "folder"}
                problem={problems.folder}
              >
                {#snippet control()}
                  <span class="folder-value">
                    <Icon name="folder" />
                    {@render folderLabel(s.downloadFolder, t("settings.folder.windows"))}
                  </span>
                  <Button
                    aria-describedby={describedBy("folder", "folder")}
                    onclick={() =>
                      chooseFolder("folder", s.downloadFolder, (downloadFolder) => ({
                        downloadFolder,
                      }))}
                  >
                    {t("settings.folder.change")}
                  </Button>
                  {#if s.downloadFolder}
                    <IconButton
                      label={t("settings.folder.reset")}
                      onclick={() => void save("folder", { downloadFolder: "" })}
                    >
                      <Icon name="close" />
                    </IconButton>
                  {/if}
                {/snippet}
              </SettingRow>
            {/if}

            {#if shown.has("categories")}
              <SettingRow
                id="categories"
                label={t("settings.categories")}
                description={t("settings.categories.description")}
                target={highlighted === "categories"}
              >
                <ul class="categories">
                  {#each s.categories as category (category.id)}
                    {@const name = categoryName(category.id)}
                    {@const field = `ext.${category.id}`}
                    <li class="category">
                      <div class="category-head">
                        <span class="category-name"
                          ><Icon name={CATEGORY_ICONS[category.id]} />{name}</span
                        >
                        <button
                          type="button"
                          class="folder-button"
                          aria-label={t("settings.category.folder.label", {
                            category: name,
                            folder: category.folder ?? t("settings.category.folder.default"),
                          })}
                          onclick={() =>
                            chooseFolder(
                              `folder.${category.id}`,
                              category.folder ?? s.downloadFolder,
                              (folder) => ({
                                category: { id: category.id, extensions: null, folder },
                              }),
                            )}
                        >
                          <Icon name="folder" />
                          {@render folderLabel(
                            category.folder,
                            t("settings.category.folder.default"),
                          )}
                        </button>
                        {#if category.folder}
                          <IconButton
                            label={t("settings.category.folder.clear")}
                            onclick={() =>
                              void save(`folder.${category.id}`, {
                                category: { id: category.id, extensions: null, folder: "" },
                              })}
                          >
                            <Icon name="close" />
                          </IconButton>
                        {/if}
                      </div>
                      {#if category.id === "other"}
                        <p class="category-note">{t("settings.category.other")}</p>
                      {:else}
                        <TextField
                          mono
                          bind:value={
                            () => extensions[category.id] ?? "",
                            (v) => (extensions = { ...extensions, [category.id]: v })
                          }
                          aria-label={t("settings.category.extensions.label", { category: name })}
                          aria-describedby={problems[field] ? `problem-${field}` : undefined}
                          placeholder={t("settings.category.extensions.hint")}
                          invalid={!!problems[field]}
                          onfocus={focusField(field)}
                          onblur={() => commitExtensions(category.id)}
                          onkeydown={(e) => commitOn(e, () => commitExtensions(category.id))}
                          oninput={() => problem(field, null)}
                        />
                      {/if}
                      {#if problems[field] || problems[`folder.${category.id}`]}
                        <p class="problem" id="problem-{field}" role="alert">
                          {problems[field] ?? problems[`folder.${category.id}`]}
                        </p>
                      {/if}
                    </li>
                  {/each}
                </ul>
              </SettingRow>
            {/if}

            {#if shown.has("connections")}
              <SettingRow
                id="connections"
                label={t("settings.connections")}
                description={t("settings.connections.description")}
                target={highlighted === "connections"}
                problem={problems.connections}
              >
                {#snippet control()}
                  <span class="narrow">
                    <TextField
                      bind:value={connections}
                      inputmode="numeric"
                      aria-labelledby="setting-connections"
                      aria-describedby={describedBy("connections", "connections")}
                      invalid={!!problems.connections}
                      onfocus={focusField("connections")}
                      onblur={commitConnections}
                      onkeydown={(e) => commitOn(e, commitConnections)}
                      oninput={() => problem("connections", null)}
                    />
                  </span>
                {/snippet}
              </SettingRow>
            {/if}

            {#if shown.has("limit")}
              <SettingRow
                id="limit"
                label={t("settings.limit")}
                description={t("settings.limit.description")}
                target={highlighted === "limit"}
                problem={problems.limit}
              >
                {#snippet control()}
                  <span class="narrow wide-unit">
                    <TextField
                      bind:value={limit}
                      inputmode="decimal"
                      unit={t("add.limit.unit")}
                      placeholder={t("add.limit.none")}
                      aria-label={t("settings.limit.input")}
                      aria-describedby={describedBy("limit", "limit")}
                      invalid={!!problems.limit}
                      onfocus={focusField("limit")}
                      onblur={commitLimit}
                      onkeydown={(e) => commitOn(e, () => void commitLimit())}
                      oninput={() => problem("limit", null)}
                    />
                  </span>
                {/snippet}
              </SettingRow>
            {/if}

            {#if shown.has("proxy")}
              <SettingRow
                id="proxy"
                label={t("settings.proxy")}
                description={t("settings.proxy.description")}
                target={highlighted === "proxy"}
                problem={problems.proxy}
              >
                {#snippet control()}
                  <Select
                    value={proxyMode}
                    aria-labelledby="setting-proxy"
                    aria-describedby={describedBy("proxy", "proxy")}
                    options={[
                      { value: "none", label: t("settings.proxy.none") },
                      { value: "system", label: t("settings.proxy.system") },
                      { value: "manual", label: t("settings.proxy.manual") },
                    ]}
                    onchange={(e) => chooseProxyMode(e.currentTarget.value as ProxyMode)}
                  />
                {/snippet}
                {#if proxyMode === "system"}
                  <div class="system">
                    <p class="note">
                      {system?.proxy
                        ? t("settings.proxy.system.uses", { proxy: system.proxy })
                        : t("settings.proxy.system.none")}
                    </p>
                    {#if system?.script}
                      <p class="note warning">
                        <Icon name="alert" />{t("settings.proxy.system.script")}
                      </p>
                    {/if}
                  </div>
                {:else if proxyMode === "manual"}
                  <div class="fields">
                    <label class="field" for="proxy-url">
                      <span class="field-label">{t("settings.proxy.url")}</span>
                      <TextField
                        id="proxy-url"
                        mono
                        bind:value={proxyUrl}
                        placeholder={t("settings.proxy.url.placeholder")}
                        aria-describedby="proxy-url-hint {problems.proxy
                          ? 'setting-proxy-problem'
                          : ''}"
                        invalid={!!problems.proxy}
                        onfocus={focusField("proxyUrl")}
                        onblur={commitProxy}
                        onkeydown={(e) => commitOn(e, commitProxy)}
                        oninput={() => problem("proxy", null)}
                      />
                      <span class="field-hint" id="proxy-url-hint"
                        >{t("settings.proxy.url.description")}</span
                      >
                    </label>
                    <label class="field" for="proxy-bypass">
                      <span class="field-label">{t("settings.proxy.bypass")}</span>
                      <TextField
                        id="proxy-bypass"
                        mono
                        bind:value={proxyBypass}
                        aria-describedby="proxy-bypass-hint"
                        onfocus={focusField("proxyBypass")}
                        onblur={commitProxy}
                        onkeydown={(e) => commitOn(e, commitProxy)}
                      />
                      <span class="field-hint" id="proxy-bypass-hint"
                        >{t("settings.proxy.bypass.description")}</span
                      >
                    </label>
                  </div>
                {/if}
              </SettingRow>
            {/if}

            {#if shown.has("shortcuts")}
              <SettingRow
                id="shortcuts"
                label={t("settings.section.shortcuts")}
                description={t("settings.shortcuts.description")}
                target={highlighted === "shortcuts"}
              >
                <dl class="shortcuts">
                  {#each SHORTCUTS as shortcut (shortcut.id)}
                    <div class="shortcut">
                      <dt>{t(shortcut.label)}</dt>
                      <dd><Keycap shortcut={t(shortcut.keys)} /></dd>
                    </div>
                  {/each}
                </dl>
              </SettingRow>
            {/if}

            {#if shown.has("about")}
              <SettingRow id="about" label={t("app.name")} target={highlighted === "about"}>
                <div class="about">
                  {#if info}
                    <p>
                      {info.debug
                        ? t("settings.about.debug", { version: info.version })
                        : t("settings.about.version", { version: info.version })}
                    </p>
                  {/if}
                  <p class="muted">{t("settings.about.license")}</p>
                  <p class="muted">{t("settings.about.trust")}</p>
                </div>
              </SettingRow>
            {/if}
          </div>
        {/each}
      </div>
    </div>
    <Toasts />
  </main>
</div>

<style>
  .settings {
    display: flex;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }

  /* The sections, where the sidebar is. */
  .nav {
    display: flex;
    flex-direction: column;
    flex: none;
    width: var(--sidebar-width);
    padding: var(--space-1) var(--space-2) var(--space-2);
    background: var(--color-chrome);
    overflow-y: auto;
  }
  .nav ul {
    display: flex;
    flex-direction: column;
    gap: 1px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .heading {
    margin: var(--space-4) 0 var(--space-1);
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
  }
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
    background: var(--color-accent);
  }
  .item[aria-current="page"] :global(.icon) {
    color: var(--color-accent);
  }
  .item-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* The content: the same sheet as the list's, with one readable column. */
  .frame {
    position: relative;
    display: flex;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    background: var(--color-content);
    border-top: 1px solid var(--color-hairline);
    border-left: 1px solid var(--color-hairline);
    border-top-left-radius: var(--radius-floating);
  }
  .content {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
  }
  .column {
    max-width: 760px;
    padding: var(--space-6) var(--space-8) var(--space-12);
  }
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .section + .section {
    margin-top: var(--space-8);
  }
  .title {
    margin: 0 0 var(--space-3);
    font-family: var(--font-display);
    font-size: var(--text-20);
    line-height: var(--leading-20);
    font-weight: var(--weight-title);
    letter-spacing: var(--tracking-20);
  }
  .none {
    margin: var(--space-8) 0;
    color: var(--color-text-muted);
  }
  .muted {
    color: var(--color-text-muted);
  }

  /* Folders. */
  .folder-value,
  .folder-button {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    max-width: 280px;
    color: var(--color-text-subtle);
  }
  .folder-button {
    height: var(--control-height);
    padding: 0 var(--space-2);
    border: 1px solid var(--color-hairline-strong);
    border-radius: var(--radius-control);
    background: var(--color-raised);
  }
  .folder-button:hover {
    background: var(--color-overlay-hover);
  }
  .path {
    display: flex;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    color: var(--color-text);
  }
  .path.muted {
    color: var(--color-text-muted);
  }
  .parent {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    direction: rtl;
    text-align: left;
    color: var(--color-text-subtle);
  }
  .parent > span {
    unicode-bidi: isolate;
  }
  .leaf {
    flex: none;
  }

  /* Categories. */
  .categories {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
    border-top: 1px solid var(--color-hairline);
  }
  .category {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) 0;
    border-bottom: 1px solid var(--color-hairline);
  }
  .category:last-child {
    border-bottom: 0;
    padding-bottom: 0;
  }
  .category-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .category-name {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    flex: 1;
    min-width: 0;
    font-weight: var(--weight-medium);
  }
  .category-name :global(.icon) {
    color: var(--color-text-muted);
  }
  .category :global(.text-field) {
    width: 100%;
  }
  .category-note {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--text-12);
  }
  .problem {
    margin: 0;
    color: var(--color-error);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }

  /* Numbers. */
  .narrow {
    display: inline-flex;
    width: 72px;
  }
  .narrow.wide-unit {
    width: 140px;
  }
  .narrow :global(.text-field) {
    width: 100%;
  }

  /* Proxy. */
  .system {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .note {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--text-12);
  }
  .note.warning :global(.icon) {
    color: var(--color-warning);
  }
  .fields {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }
  .field-label {
    color: var(--color-text-muted);
    font-size: var(--text-12);
  }
  .field-hint {
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }

  /* Shortcuts. */
  .shortcuts {
    display: grid;
    grid-template-columns: 1fr 1fr;
    column-gap: var(--space-6);
    margin: 0;
  }
  .shortcut {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-height: 32px;
    border-bottom: 1px solid var(--color-hairline);
  }
  .shortcut dt {
    color: var(--color-text-muted);
  }
  .shortcut dd {
    margin: 0;
  }

  .about p {
    margin: 0 0 var(--space-1);
  }

  /* The icon rail, as the sidebar does. */
  @media (max-width: 899px) {
    .nav {
      width: var(--sidebar-rail-width);
      padding-inline: var(--space-1);
    }
    .item {
      justify-content: center;
      padding: 0;
    }
    .item-label {
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
    .column {
      padding: var(--space-5) var(--space-5) var(--space-8);
    }
  }
  @media (max-width: 719px) {
    .fields,
    .shortcuts {
      grid-template-columns: 1fr;
    }
  }

  @media (forced-colors: active) {
    .item[aria-current="page"] {
      outline: 1px solid Highlight;
    }
  }
</style>
