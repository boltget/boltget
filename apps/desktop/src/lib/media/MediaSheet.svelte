<!--
  The media sheet (docs/APP-DESIGN.md, "Video as built"): what "Look for
  media on this page" opens. It looks at the page with yt-dlp and shows the
  picker (docs/VIDEO.md, "Format and quality picker UX"): the page's
  thumbnail, title, uploader and length; the presets with resolution,
  container, codecs and size; Prefer MP4; subtitles; and, under Advanced,
  every format with a free format expression. One decision: Enter
  downloads the chosen preset into the Video folder.

  When yt-dlp or FFmpeg is missing it first asks to install them (VIDEO.md,
  "First-use install flow"), with their sizes from the release metadata and
  the one line about rights, shows the install's progress, then looks at
  the page by itself.

  A page the browser extension sent ("Download video from this page") comes
  with its browser context, which the probe and the download use, and the
  media the extension saw the page load: when yt-dlp finds nothing, the
  sheet offers those (VIDEO.md, "Hints"): a stream's manifest goes to
  yt-dlp, a direct file to the add sheet.

  A native modal <dialog>, like the add sheet: it traps focus and Esc
  closes it.
-->
<script lang="ts">
  import Button from "@boltget/ui/components/Button.svelte";
  import IconButton from "@boltget/ui/components/IconButton.svelte";
  import Select from "@boltget/ui/components/Select.svelte";
  import Switch from "@boltget/ui/components/Switch.svelte";
  import TextField from "@boltget/ui/components/TextField.svelte";
  import { t } from "@boltget/ui/messages";
  import { untrack } from "svelte";
  import { app } from "../app.svelte";
  import type {
    BrowserMediaPage,
    MediaContainer,
    MediaHintInfo,
    MediaInstallEvent,
    MediaSummary,
    MediaTool,
    MediaToolSize,
    UiError,
  } from "../bindings";
  import { downloads, errorText } from "../engine";
  import { splitFolder } from "../format";
  import { trapTab } from "../focus";
  import Icon from "../Icon.svelte";
  import { folderFor } from "../settings/model";
  import {
    expressionFor,
    expressionIsAudio,
    expressionProblem,
    formatRows,
    hintLine,
    initialPreset,
    installLines,
    installSummary,
    mediaLength,
    offeredHints,
    pickerNotice,
    presetDetails,
    presetName,
    presetsFor,
    subtitleOptions,
  } from "./picker";

  const EXIT_MS = 150;

  type Phase = "looking" | "install" | "installing" | "picker" | "failed";

  let dialog: HTMLDialogElement | undefined = $state();
  let open = $state(false);
  let closing = $state(false);
  let lastKey = -1;

  let url = $state("");
  /** The page as the browser extension sent it, if it did. */
  let browserPage = $state.raw<BrowserMediaPage | null>(null);
  let phase = $state<Phase>("looking");
  let media = $state.raw<MediaSummary | null>(null);
  let failure = $state.raw<UiError | null>(null);
  let thumbnail = $state<string | null>(null);

  // The install sheet.
  let plan = $state.raw<MediaToolSize[] | null>(null);
  let planFailed = $state(false);
  let events = $state.raw<MediaInstallEvent[]>([]);
  let installError = $state<string | null>(null);

  // The picker.
  let container = $state<MediaContainer>("mp4");
  let preset = $state<string | null>(null);
  let subtitle = $state("none");
  let subtitleMode = $state<"embed" | "file">("embed");
  let advanced = $state(false);
  let videoId = $state<string | null>(null);
  let audioId = $state<string | null>(null);
  let expression = $state("");
  let expressionError = $state<string | null>(null);
  let downloadsFolder = $state<string | null>(null);
  let submitting = $state(false);
  let submitError = $state<string | null>(null);

  const reduced =
    typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

  // Derived -----------------------------------------------------------------

  const notice = $derived(media ? pickerNotice(media) : null);
  const list = $derived(media ? presetsFor(media, container) : []);
  const chosen = $derived(list.find((p) => p.preset === preset) ?? null);
  const subtitles = $derived(media ? subtitleOptions(media.subtitles) : []);
  const rows = $derived(media ? formatRows(media.formats) : []);
  const typed = $derived(advanced && expression.trim() !== "");
  const audioOnly = $derived(
    typed ? expressionIsAudio(expression.trim(), media?.formats ?? []) : !!chosen?.audioOnly,
  );
  const folder = $derived(
    folderFor("video.mp4", downloads.settings, downloadsFolder) ?? downloadsFolder,
  );
  const install = $derived(plan ? installSummary(plan) : null);
  const progress = $derived(installLines(plan ?? [], events));
  const meta = $derived.by(() => {
    if (!media) return [];
    return [
      ...(media.uploader ? [media.uploader] : []),
      ...(media.duration !== null ? [mediaLength(media.duration)] : []),
    ];
  });
  const hints = $derived(
    browserPage && (phase === "failed" || phase === "picker")
      ? offeredHints(browserPage.hints, phase === "failed" ? "failed" : notice, url)
      : [],
  );
  let hintBusy = $state(false);
  const canDownload = $derived(
    phase === "picker" && notice === null && (typed || chosen !== null) && !submitting,
  );

  // Opening and closing -------------------------------------------------------

  $effect(() => {
    const request = app.media;
    untrack(() => {
      if (request && (!open || request.key !== lastKey)) {
        show(request.url, request.browser, request.key);
      } else if (!request && open) hide();
    });
  });

  function show(page: string, browser: BrowserMediaPage | null, key: number) {
    lastKey = key;
    open = true;
    closing = false;
    url = page;
    browserPage = browser;
    hintBusy = false;
    media = null;
    failure = null;
    thumbnail = null;
    plan = null;
    planFailed = false;
    events = [];
    installError = null;
    container = downloads.settings.video.container;
    preset = null;
    subtitle = "none";
    subtitleMode = "embed";
    advanced = false;
    videoId = null;
    audioId = null;
    expression = "";
    expressionError = null;
    submitting = false;
    submitError = null;
    void downloads.addDefaults().then((r) => {
      if (r.status === "ok") downloadsFolder = r.data.downloadsFolder;
    });
    queueMicrotask(() => {
      if (!dialog?.open) dialog?.showModal();
      dialog?.focus();
    });
    void look();
  }

  function hide() {
    if (!dialog?.open) {
      open = false;
      return;
    }
    closing = true;
    setTimeout(
      () => {
        dialog?.close();
        open = false;
        closing = false;
      },
      reduced ? 0 : EXIT_MS,
    );
  }

  function cancel(event?: Event) {
    event?.preventDefault();
    // An install runs on; only the sheet goes.
    app.closeMedia();
  }

  // Looking -------------------------------------------------------------------

  async function look() {
    const page = url;
    phase = "looking";
    failure = null;
    const result = await downloads.probeMedia(page, browserPage?.page ?? null);
    if (page !== url || !open) return;
    if (result.status === "error") {
      failure = result.error;
      phase = "failed";
      return;
    }
    if (result.data.kind === "notInstalled") {
      phase = "install";
      void loadPlan();
      return;
    }
    const found = result.data.media;
    media = found;
    preset = initialPreset(presetsFor(found, container), null, found.remembered);
    phase = "picker";
    focusFirst();
    if (found.thumbnail) {
      void downloads.mediaThumbnail(found.thumbnail).then((r) => {
        if (r.status === "ok" && media === found) thumbnail = r.data;
      });
    }
  }

  function focusFirst() {
    queueMicrotask(() => {
      const first =
        dialog?.querySelector<HTMLInputElement>(".preset input:checked") ??
        dialog?.querySelector<HTMLElement>(".foot button");
      first?.focus();
    });
  }

  // Installing ----------------------------------------------------------------

  async function loadPlan() {
    const result = await downloads.mediaInstallPlan();
    if (phase !== "install" && phase !== "installing") return;
    if (result.status === "ok") plan = result.data;
    else planFailed = true;
  }

  async function installTools() {
    phase = "installing";
    installError = null;
    events = [];
    const result = await downloads.installMedia((event) => (events = [...events, event]));
    if (!open) return;
    if (result.status === "error") {
      installError = errorText(result.error);
      phase = "install";
      return;
    }
    // The original request continues by itself (VIDEO.md).
    void look();
  }

  // Hints ---------------------------------------------------------------------

  /**
   * A hint the page loaded: a stream's manifest is looked at by yt-dlp like
   * a page (with the browser's context, the page as referrer); a direct
   * file goes to the add sheet, its request waiting there.
   */
  async function useHint(hint: MediaHintInfo) {
    if (!browserPage || hintBusy) return;
    if (hint.kind !== "file") {
      url = hint.url;
      media = null;
      thumbnail = null;
      preset = null;
      void look();
      return;
    }
    hintBusy = true;
    submitError = null;
    const result = await downloads.mediaHintLink(browserPage.page, hint.url);
    hintBusy = false;
    if (result.status === "error") {
      submitError = errorText(result.error);
      return;
    }
    app.closeMedia();
    app.openAdd(hint.url);
  }

  function openProject(tool: MediaTool) {
    void downloads.openMediaProject(tool);
  }

  // Choosing ------------------------------------------------------------------

  function setContainer(mp4: boolean) {
    container = mp4 ? "mp4" : "original";
    if (media) preset = initialPreset(presetsFor(media, container), preset, media.remembered);
  }

  function pickVideo(id: string | null) {
    videoId = id;
    expression = expressionFor(videoId, audioId);
    expressionError = null;
  }

  function pickAudio(id: string | null) {
    audioId = id;
    expression = expressionFor(videoId, audioId);
    expressionError = null;
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (!media || !canDownload) return;
    let request;
    if (typed) {
      const problem = expressionProblem(expression);
      if (problem) {
        expressionError = problem;
        return;
      }
      request = { expression: expression.trim(), preset: null, size: null };
    } else if (chosen) {
      request = { expression: chosen.expression, preset: chosen.preset, size: chosen.size };
    } else return;
    const option = subtitles.find((o) => o.value === subtitle);
    submitting = true;
    submitError = null;
    const result = await downloads.addMedia({
      url: media.url,
      container,
      title: media.title,
      site: media.site,
      page: browserPage?.page ?? null,
      subtitles: option
        ? {
            languages: [option.code],
            automatic: option.automatic,
            embed: subtitleMode === "embed" && !audioOnly,
          }
        : null,
      ...request,
    });
    submitting = false;
    if (result.status === "error") {
      submitError = errorText(result.error);
      return;
    }
    app.select([result.data.id]);
    app.announce(t("media.added", { name: media.title ?? media.url }));
    app.closeMedia();
  }
</script>

{#snippet hintList()}
  {#if hints.length}
    <section class="hints" aria-labelledby="media-hints">
      <div class="option-text">
        <span class="label" id="media-hints">{t("media.hints")}</span>
        <span class="hint">{t("media.hints.description")}</span>
      </div>
      <ul class="hint-list">
        {#each hints as h (h.url)}
          {@const line = hintLine(h)}
          <li>
            <span class="hint-text">
              <span class="hint-name">{line.name}</span>
              <span class="hint-details">{@render dots(line.details)}</span>
            </span>
            <Button
              disabled={hintBusy}
              aria-label={t("media.hint.use_label", {
                name: line.name.toLowerCase(),
                host: line.host,
              })}
              onclick={() => void useHint(h)}>{t("media.hint.use")}</Button
            >
          </li>
        {/each}
      </ul>
    </section>
  {/if}
{/snippet}

{#snippet dots(parts: string[])}
  {#each parts as part, i (i)}{#if i}<span class="dot" aria-hidden="true">·</span>{/if}<span
      >{part}</span
    >{/each}
{/snippet}

<dialog
  bind:this={dialog}
  use:trapTab
  class="sheet"
  class:closing
  class:reduced
  class:wide={advanced && phase === "picker"}
  aria-labelledby="media-title"
  tabindex="-1"
  oncancel={cancel}
  onclick={(event) => {
    if (event.target === dialog && phase !== "installing") cancel();
  }}
>
  {#if open}
    <form class="body" onsubmit={submit} novalidate>
      <header class="head">
        <h2 id="media-title" class="title">
          {phase === "install" || phase === "installing"
            ? t("media.install.title")
            : t("media.title")}
        </h2>
        <IconButton label={t("media.close")} onclick={() => cancel()}>
          <Icon name="close" />
        </IconButton>
      </header>

      <div class="status" aria-live="polite">
        {#if phase === "looking"}
          <div class="card quiet">
            <span class="thumb empty"><Icon name="video" size={20} /></span>
            <div class="card-text">
              <p class="page" title={url}>{url}</p>
              <p class="meta"><span class="probing">{t("media.looking")}</span></p>
            </div>
          </div>
        {:else if phase === "failed"}
          <p class="line warning">
            <Icon name="alert" size={14} />
            <span
              >{failure?.key === "error.media.extraction"
                ? t("media.failed")
                : errorText(failure ?? { key: "error.unknown" })}</span
            >
          </p>
          {#if failure?.message}
            <details class="technical">
              <summary>{t("media.details")}</summary>
              <code>{failure.message}</code>
            </details>
          {/if}
          {@render hintList()}
        {:else if phase === "install" || phase === "installing"}
          <p class="text">{t("media.install.body")}</p>
          {#if phase === "install"}
            <ul class="tools">
              {#if install}
                {#each install.lines as line (line.tool)}
                  <li>
                    <button
                      type="button"
                      class="project"
                      title={t("media.install.project", { name: line.name })}
                      onclick={() => openProject(line.tool)}
                      >{t("media.install.line", {
                        name: line.name,
                        version: line.version,
                      })}</button
                    >
                    <span class="size">{line.size}</span>
                  </li>
                {/each}
                {#if install.lines.length > 1}
                  <li class="total">
                    <span>{t("media.install.total", { size: install.total })}</span>
                  </li>
                {/if}
              {:else if planFailed}
                <li class="muted">{t("media.install.sizes_failed")}</li>
              {:else}
                <li class="muted"><span class="probing">{t("media.install.sizes")}</span></li>
              {/if}
            </ul>
          {:else}
            <ul class="tools progress" aria-label={t("media.install.installing")}>
              {#each progress as line (line.tool)}
                <li>
                  <span class="tool-name">{line.name}</span>
                  <span class="size" class:ready={line.done}>
                    {#if line.done}<Icon name="check" size={14} />{/if}{line.text}
                  </span>
                  <span
                    class="bar"
                    role="progressbar"
                    aria-label={line.name}
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={line.done
                      ? 100
                      : line.fraction !== null
                        ? Math.round(line.fraction * 100)
                        : undefined}
                    aria-valuetext={line.text}
                  >
                    <span
                      class="fill"
                      class:indeterminate={line.started && !line.done && line.fraction === null}
                      style:transform="scaleX({line.done ? 1 : (line.fraction ?? 0)})"
                    ></span>
                  </span>
                </li>
              {/each}
            </ul>
          {/if}
          <p class="rights">{t("media.install.rights")}</p>
          {#if installError}
            <p class="line error" role="alert">
              <Icon name="alert" size={14} /><span>{installError}</span>
            </p>
          {/if}
        {:else if media}
          <div class="card">
            {#if thumbnail}
              <img class="thumb" src={thumbnail} alt="" />
            {:else}
              <span class="thumb empty"><Icon name="video" size={20} /></span>
            {/if}
            <div class="card-text">
              <p class="page" title={media.title ?? media.url}>{media.title ?? media.url}</p>
              {#if meta.length}<p class="meta">{@render dots(meta)}</p>{/if}
            </div>
          </div>

          {#if notice === "playlist"}
            <p class="line info">
              <Icon name="info" size={14} />
              <span>{t("media.playlist", { count: media.entryCount })}</span>
            </p>
          {:else if notice === "drm"}
            <p class="line warning">
              <Icon name="lock" size={14} /><span>{t("media.drm")}</span>
            </p>
          {:else if notice === "none"}
            <p class="line info"><Icon name="info" size={14} /><span>{t("media.none")}</span></p>
            {@render hintList()}
          {:else}
            <fieldset class="presets" disabled={typed}>
              <legend class="label">{t("media.presets")}</legend>
              {#each list as p (p.preset)}
                <label class="preset" class:checked={preset === p.preset && !typed}>
                  <input
                    type="radio"
                    name="media-preset"
                    value={p.preset}
                    checked={preset === p.preset}
                    aria-labelledby="media-preset-{p.preset}"
                    aria-describedby="media-preset-{p.preset}-details"
                    onchange={() => (preset = p.preset)}
                  />
                  <span class="preset-name" id="media-preset-{p.preset}"
                    >{presetName(p.preset)}</span
                  >
                  <span class="preset-details" id="media-preset-{p.preset}-details"
                    >{@render dots(presetDetails(p))}</span
                  >
                </label>
              {/each}
            </fieldset>

            <div class="option">
              <div class="option-text">
                <span class="label" id="media-mp4">{t("media.prefer_mp4")}</span>
                <span class="hint" id="media-mp4-hint">{t("media.prefer_mp4.description")}</span>
              </div>
              <Switch
                checked={container === "mp4"}
                aria-labelledby="media-mp4"
                aria-describedby="media-mp4-hint"
                onchange={(e) => setContainer(e.currentTarget.checked)}
              />
            </div>

            {#if subtitles.length}
              <div class="option">
                <span class="label" id="media-subs">{t("media.subtitles")}</span>
                <div class="subs">
                  <Select
                    bind:value={subtitle}
                    aria-labelledby="media-subs"
                    options={[
                      { value: "none", label: t("media.subtitles.none") },
                      ...subtitles.map((o) => ({ value: o.value, label: o.label })),
                    ]}
                  />
                  {#if subtitle !== "none" && !audioOnly}
                    <Select
                      bind:value={subtitleMode}
                      aria-label={t("media.subtitles.how")}
                      options={[
                        { value: "embed", label: t("media.subtitles.embed") },
                        { value: "file", label: t("media.subtitles.file") },
                      ]}
                    />
                  {/if}
                </div>
              </div>
            {/if}

            <div class="advanced">
              <button
                type="button"
                class="disclosure"
                aria-expanded={advanced}
                aria-controls="media-advanced"
                onclick={() => (advanced = !advanced)}
              >
                <span class="chevron" class:open={advanced}><Icon name="chevron" size={14} /></span>
                {t("media.advanced")}
              </button>
              {#if advanced}
                <div id="media-advanced" class="advanced-body">
                  <p class="hint">{t("media.advanced.description")}</p>
                  <div class="table-wrap">
                    <table class="formats">
                      <caption class="visually-hidden">{t("media.format.table")}</caption>
                      <thead>
                        <tr>
                          <th scope="col">{t("media.format.video")}</th>
                          <th scope="col">{t("media.format.audio")}</th>
                          <th scope="col">{t("media.format.id")}</th>
                          <th scope="col">{t("media.format.resolution")}</th>
                          <th scope="col" class="num">{t("media.format.fps")}</th>
                          <th scope="col">{t("media.format.codec")}</th>
                          <th scope="col" class="num">{t("media.format.bitrate")}</th>
                          <th scope="col" class="num">{t("media.format.size")}</th>
                          <th scope="col">{t("media.format.protocol")}</th>
                        </tr>
                      </thead>
                      <tbody>
                        <tr>
                          <td
                            ><input
                              type="radio"
                              name="media-video"
                              aria-label={t("media.format.no_video")}
                              checked={videoId === null}
                              onchange={() => pickVideo(null)}
                            /></td
                          >
                          <td
                            ><input
                              type="radio"
                              name="media-audio"
                              aria-label={t("media.format.no_audio")}
                              checked={audioId === null}
                              onchange={() => pickAudio(null)}
                            /></td
                          >
                          <td colspan="7" class="muted">–</td>
                        </tr>
                        {#each rows as r (r.id)}
                          <tr class:picked={videoId === r.id || audioId === r.id}>
                            <td>
                              {#if r.video}<input
                                  type="radio"
                                  name="media-video"
                                  aria-label={t("media.format.pick_video", { id: r.id })}
                                  checked={videoId === r.id}
                                  onchange={() => pickVideo(r.id)}
                                />{/if}
                            </td>
                            <td>
                              {#if r.audio}<input
                                  type="radio"
                                  name="media-audio"
                                  aria-label={t("media.format.pick_audio", { id: r.id })}
                                  checked={audioId === r.id}
                                  onchange={() => pickAudio(r.id)}
                                />{/if}
                            </td>
                            <td class="mono">{r.id}</td>
                            <td>{r.resolution}</td>
                            <td class="num">{r.fps}</td>
                            <td>{r.codec}</td>
                            <td class="num">{r.bitrate}</td>
                            <td class="num">{r.size}</td>
                            <td>{r.protocol}</td>
                          </tr>
                        {/each}
                      </tbody>
                    </table>
                  </div>
                  <label class="label" for="media-expression">{t("media.expression")}</label>
                  <TextField
                    id="media-expression"
                    mono
                    bind:value={expression}
                    invalid={!!expressionError}
                    spellcheck={false}
                    autocomplete="off"
                    aria-describedby="media-expression-hint"
                    oninput={() => (expressionError = null)}
                  />
                  <p class="hint" id="media-expression-hint" class:error={!!expressionError}>
                    {expressionError ?? t("media.expression.hint")}
                  </p>
                </div>
              {/if}
            </div>

            {#if folder}
              {@const parts = splitFolder(folder)}
              <p class="line muted" title={folder}>
                <Icon name="folder" size={14} />
                <span class="folder"
                  >{t("media.destination", { folder: parts.leaf || folder })}</span
                >
              </p>
            {/if}
          {/if}
        {/if}
      </div>

      <footer class="foot">
        <p class="foot-note" class:error={!!submitError} aria-live="polite">{submitError ?? ""}</p>
        {#if phase === "install"}
          <Button onclick={() => cancel()}>{t("action.cancel")}</Button>
          <Button variant="primary" onclick={installTools}>{t("media.install.action")}</Button>
        {:else if phase === "installing"}
          <Button onclick={() => cancel()}>{t("media.close")}</Button>
        {:else if phase === "failed"}
          <Button onclick={() => cancel()}>{t("action.cancel")}</Button>
          <Button variant="primary" onclick={() => void look()}>{t("media.retry")}</Button>
        {:else}
          <Button onclick={() => cancel()}>{t("action.cancel")}</Button>
          <Button variant="primary" type="submit" disabled={!canDownload}
            >{t("media.download")}</Button
          >
        {/if}
      </footer>
    </form>
  {/if}
</dialog>

<style>
  .sheet {
    width: min(600px, calc(100vw - 48px));
    max-height: calc(100vh - 96px);
    margin: 72px auto auto;
    padding: 0;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-floating);
    background: var(--color-raised);
    color: var(--color-text);
    box-shadow: var(--elevation-floating);
    overflow-y: auto;
    outline: none;
  }
  .sheet.wide {
    width: min(780px, calc(100vw - 48px));
  }
  .sheet[open] {
    animation: enter var(--duration-pane) var(--ease-enter);
  }
  .sheet.closing {
    animation: leave var(--duration-small-exit) var(--ease-exit) forwards;
  }
  .sheet::backdrop {
    background: var(--color-scrim);
    animation: fade var(--duration-pane) var(--ease-enter);
  }
  .sheet.closing::backdrop {
    animation: fade var(--duration-small-exit) var(--ease-exit) reverse forwards;
  }
  .sheet.reduced,
  .sheet.reduced::backdrop {
    animation: none;
  }
  @keyframes enter {
    from {
      opacity: 0;
      transform: translateY(-8px);
    }
  }
  @keyframes leave {
    to {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }

  .body {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4) var(--space-5) var(--space-5);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin: calc(-1 * var(--space-1)) calc(-1 * var(--space-2)) 0 0;
  }
  .title {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-15);
    line-height: var(--leading-15);
    font-weight: var(--weight-title);
    letter-spacing: var(--tracking-15);
  }
  .status {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .text {
    margin: 0;
    color: var(--color-text-muted);
  }

  /* The page: thumbnail, title, uploader and length. */
  .card {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: 60px;
    padding: var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-floating);
    background: var(--color-content);
  }
  .thumb {
    flex: none;
    width: 96px;
    height: 54px;
    border-radius: var(--radius-control);
    object-fit: cover;
    background: var(--color-overlay-hover);
  }
  .thumb.empty {
    display: inline-grid;
    place-items: center;
    color: var(--color-text-subtle);
  }
  .card-text {
    flex: 1;
    min-width: 0;
  }
  .page {
    margin: 0;
    font-size: var(--text-13);
    line-height: var(--leading-13);
    font-weight: var(--weight-medium);
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow-wrap: anywhere;
  }
  .card.quiet .page {
    color: var(--color-text-muted);
    font-family: var(--font-mono);
    font-size: var(--text-12);
    font-weight: var(--weight-secondary);
  }
  .meta {
    margin: 2px 0 0;
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dot {
    margin: 0 6px;
  }
  .probing {
    animation: breathe 1.4s ease-in-out infinite;
  }
  @keyframes breathe {
    50% {
      opacity: 0.45;
    }
  }

  .line {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    margin: 0;
    font-size: var(--text-12);
    line-height: var(--leading-12);
    color: var(--color-text-muted);
  }
  .line :global(.icon) {
    flex: none;
  }
  .line.warning :global(.icon) {
    color: var(--color-warning);
  }
  .line.info :global(.icon),
  .line.muted {
    color: var(--color-text-subtle);
  }
  .line.error {
    color: var(--color-error);
  }
  .folder {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .technical summary {
    cursor: default;
    font-size: var(--text-12);
    color: var(--color-text-subtle);
  }
  .technical code {
    display: block;
    margin-top: var(--space-1);
    padding: var(--space-2);
    border-radius: var(--radius-control);
    background: var(--color-content);
    font-family: var(--font-mono);
    font-size: var(--text-12);
    overflow-wrap: anywhere;
    user-select: text;
  }

  /* Presets: one obvious choice, a short list. */
  .presets {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-floating);
    background: var(--color-content);
    overflow: hidden;
  }
  .presets[disabled] {
    opacity: 0.55;
  }
  .label {
    font-size: var(--text-12);
    font-weight: var(--weight-medium);
    color: var(--color-text-muted);
  }
  legend.label {
    float: left;
    width: 100%;
    padding: var(--space-2) var(--space-3) var(--space-1);
  }
  .preset {
    display: grid;
    grid-template-columns: auto 1fr;
    column-gap: var(--space-2);
    align-items: center;
    clear: both;
    padding: var(--space-2) var(--space-3);
    border-top: 1px solid var(--color-hairline);
  }
  .preset:hover {
    background: var(--color-overlay-hover);
  }
  .preset.checked {
    background: var(--color-accent-tint);
  }
  .preset input {
    grid-row: span 2;
    margin: 0;
    accent-color: var(--color-accent);
  }
  .preset-name {
    font-weight: var(--weight-medium);
  }
  .preset-details {
    grid-column: 2;
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }

  .option {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .option-text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .hint {
    margin: 0;
    font-size: var(--text-12);
    line-height: var(--leading-12);
    color: var(--color-text-subtle);
  }
  .hint.error {
    color: var(--color-error);
  }
  .subs {
    display: flex;
    gap: var(--space-2);
  }

  .disclosure {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: 2px 0;
    border: 0;
    background: none;
    color: var(--color-text-muted);
    font-weight: var(--weight-medium);
  }
  .disclosure:hover {
    color: var(--color-text);
  }
  .chevron {
    display: inline-flex;
    transition: transform var(--duration-small) var(--ease-enter);
  }
  .chevron.open {
    transform: rotate(90deg);
  }
  .advanced-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
  .advanced-body :global(.text-field) {
    display: flex;
  }
  .table-wrap {
    max-height: 220px;
    overflow: auto;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-control);
    background: var(--color-content);
  }
  .formats {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-12);
    line-height: var(--leading-12);
    white-space: nowrap;
  }
  .formats th {
    position: sticky;
    top: 0;
    padding: var(--space-1) var(--space-2);
    background: var(--color-raised);
    color: var(--color-text-subtle);
    font-weight: var(--weight-medium);
    text-align: left;
  }
  .formats td {
    padding: 3px var(--space-2);
    border-top: 1px solid var(--color-hairline);
  }
  .formats .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .formats .mono {
    font-family: var(--font-mono);
  }
  .formats tr.picked td {
    background: var(--color-accent-tint);
  }
  .formats input {
    margin: 0;
    accent-color: var(--color-accent);
  }
  .muted {
    color: var(--color-text-subtle);
  }

  /* Hints: what the page loaded, when yt-dlp found nothing. */
  .hints {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .hint-list {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-floating);
    background: var(--color-content);
    list-style: none;
    overflow: hidden;
  }
  .hint-list li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
  }
  .hint-list li + li {
    border-top: 1px solid var(--color-hairline);
  }
  .hint-text {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-width: 0;
  }
  .hint-name {
    font-weight: var(--weight-medium);
  }
  .hint-details {
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* The install sheet. */
  .tools {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: var(--space-1) 0;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-floating);
    background: var(--color-content);
    list-style: none;
  }
  .tools li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-2);
    padding: var(--space-2) var(--space-3);
  }
  .tools li.muted {
    color: var(--color-text-subtle);
  }
  .tools .total {
    justify-content: flex-end;
    border-top: 1px solid var(--color-hairline);
    color: var(--color-text-subtle);
    font-size: var(--text-12);
  }
  .project {
    padding: 0;
    border: 0;
    background: none;
    color: var(--color-accent);
    font-weight: var(--weight-medium);
  }
  .project {
    text-decoration: underline;
    text-decoration-color: var(--color-hairline-strong);
    text-underline-offset: 3px;
  }
  .project:hover {
    text-decoration-color: currentColor;
  }
  .tool-name {
    font-weight: var(--weight-medium);
  }
  .size {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    margin-left: auto;
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    font-variant-numeric: tabular-nums;
  }
  .size.ready {
    color: var(--color-success);
  }
  .bar {
    flex-basis: 100%;
    height: 3px;
    border-radius: 2px;
    background: var(--color-overlay-hover);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    background: var(--color-accent);
    transform-origin: left;
    transition: transform var(--duration-small) linear;
  }
  .fill.indeterminate {
    width: 30%;
    transform: none !important;
    animation: slide 1.2s ease-in-out infinite;
  }
  @keyframes slide {
    from {
      margin-left: -30%;
    }
    to {
      margin-left: 100%;
    }
  }
  .rights {
    margin: 0;
    font-size: var(--text-12);
    line-height: var(--leading-12);
    color: var(--color-text-subtle);
  }

  .foot {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
  .foot-note {
    flex: 1;
    min-width: 0;
    margin: 0;
    font-size: var(--text-12);
    color: var(--color-text-subtle);
  }
  .foot-note.error {
    color: var(--color-error);
  }

  @media (prefers-reduced-motion: reduce) {
    .probing,
    .fill.indeterminate {
      animation: none;
    }
    .fill.indeterminate {
      width: 100%;
      opacity: 0.4;
      margin-left: 0;
    }
    .chevron,
    .fill {
      transition: none;
    }
  }
</style>
