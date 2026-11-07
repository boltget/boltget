<!--
  The details pane (APP-DESIGN #10 to #12, Ctrl+I): all the depth in one
  place, never a window per download. The top repeats the website's hero
  card (large tabular stats, the live file map, a throughput sparkline);
  below come the connections at work, the file (path, link, checksum,
  resume) and the per-download speed limit. A problem says what happened and
  what to do; HTTP codes stay under "Technical details".
-->
<script lang="ts">
  import Button from "@boltget/ui/components/Button.svelte";
  import IconButton from "@boltget/ui/components/IconButton.svelte";
  import { t, type PlainMessageKey } from "@boltget/ui/messages";
  import { untrack } from "svelte";
  import { app } from "../app.svelte";
  import type { DownloadDetails } from "../bindings";
  import { clock } from "../clock.svelte";
  import { downloads, errorText } from "../engine";
  import {
    bytesParts,
    displayName,
    formatBytes,
    formatDuration,
    formatSpeed,
    formatWhen,
    hostOf,
    pathPieces,
    percent,
    progressParts,
    secondsLeft,
    speedParts,
    stableEta,
    type Parts,
  } from "../format";
  import Icon, { type IconName } from "../Icon.svelte";
  import { popupMenu } from "../menu";
  import { primaryAction, type ActionId } from "../list/actions";
  import { categoryIcon } from "../list/category";
  import { isRunning, needsAttention } from "../list/model";
  import { rowMenu } from "../list/rowMenu";
  import { connections as connectionsOf } from "./fileMap";
  import FileMap from "./FileMap.svelte";
  import Sparkline from "./Sparkline.svelte";

  const MIB = 1024 * 1024;
  const SAMPLES = 120;

  const ids = $derived([...app.selection.ids]);
  const d = $derived(ids.length === 1 ? downloads.get(ids[0] as number) : undefined);

  // The pane's download streams its file map; nothing else does.
  $effect(() => {
    const id = app.paneOpen && d ? d.id : null;
    untrack(() => void downloads.focus(id));
  });

  // Checksum, links and the partial file: fetched when the download or its
  // state changes (a finished download has its digest).
  let info = $state<DownloadDetails | null>(null);
  $effect(() => {
    if (!d) return;
    const id = d.id;
    void d.state;
    void downloads.details(id).then((result) => {
      if (result.status === "ok" && d?.id === id) info = result.data;
    });
  });

  // Speed samples at 2 Hz for the sparkline; digits on the same beat.
  let samples = $state<number[]>([]);
  let bps = $state(0);
  let eta = $state<number | null>(null);
  let sampledId = -1;
  $effect(() => {
    void clock.beat;
    untrack(() => {
      if (!d) return;
      if (d.id !== sampledId) {
        sampledId = d.id;
        samples = [];
        eta = null;
      }
      bps = d.bps;
      if (isRunning(d.state)) {
        samples = [...samples.slice(-(SAMPLES - 1)), d.bps];
        eta = stableEta(eta, secondsLeft(d.downloaded, d.total, d.bps));
      } else eta = null;
    });
  });

  const ACTION_ICONS: Partial<Record<ActionId, IconName>> = {
    pause: "pause",
    resume: "play",
    retry: "retry",
    restart: "retry",
    refresh: "link",
    open: "open",
  };

  const name = $derived(d ? displayName(d.fileName, d.url) : "");
  const peak = $derived(samples.length ? Math.max(...samples) : 0);
  const running = $derived(d ? isRunning(d.state) : false);
  const complete = $derived(d?.state === "completed");
  const primary = $derived(d ? primaryAction(d.state) : "pause");
  const pct = $derived(d ? percent(d.downloaded, d.total) : null);
  const workers = $derived(d && running ? connectionsOf(d.segments, d.total) : []);
  const refreshed = $derived(info?.sources.filter((s) => s.role !== "primary").at(-1));

  const checksumText = $derived.by(() => {
    const c = info?.checksum;
    if (!c) return t("details.checksum.none");
    const algorithm = c.algorithm.toUpperCase().replace("SHA", "SHA-");
    if (c.actual === null) return t("details.checksum.pending", { algorithm });
    return c.actual === c.expected
      ? t("details.checksum.verified", { algorithm })
      : t("details.checksum.mismatch", { algorithm });
  });
  const verified = $derived(
    !!info?.checksum?.actual && info.checksum.actual === info.checksum.expected,
  );

  const mapNote = $derived.by(() => {
    if (!d) return "";
    if (complete) return verified ? checksumText : t("details.map.settled");
    if (running && d.connections > 0) return t("details.map.note", { count: d.connections });
    return pct === null ? "" : t("percent", { value: pct });
  });

  // Copy buttons answer with a check for a moment.
  let copied = $state<string | null>(null);
  async function copy(key: string, text: string) {
    try {
      await navigator.clipboard.writeText(text);
      copied = key;
      setTimeout(() => {
        if (copied === key) copied = null;
      }, 1200);
    } catch {
      app.toast(t("error.clipboard"));
    }
  }

  // Refresh link (#12).
  let freshUrl = $state("");
  let refreshInput: HTMLInputElement | undefined = $state();
  let refreshError = $state<string | null>(null);
  $effect(() => {
    if (app.refreshing !== null && d && app.refreshing === d.id && refreshInput) {
      refreshInput.focus();
      app.refreshing = null;
    }
  });
  async function submitRefresh(event: SubmitEvent) {
    event.preventDefault();
    if (!d || !freshUrl.trim()) return;
    refreshError = null;
    const result = await downloads.refreshLink(d.id, freshUrl.trim());
    if (result.status === "ok") {
      freshUrl = "";
      app.toast(t("details.refresh.done"));
    } else refreshError = errorText(result.error);
  }

  // The per-download speed limit, in MiB/s; applies on Enter or leaving.
  let limitText = $state("");
  $effect(() => {
    const limit = d?.speedLimit ?? null;
    untrack(() => (limitText = limit === null ? "" : String(Number((limit / MIB).toFixed(2)))));
  });
  function applyLimit() {
    if (!d) return;
    const value = Number.parseFloat(limitText.replace(",", "."));
    const bytes = Number.isFinite(value) && value > 0 ? Math.round(value * MIB) : null;
    if (bytes === d.speedLimit) return;
    void downloads.setSpeedLimit({ kind: "download", id: d.id }, bytes);
  }

  function more(event: MouseEvent) {
    if (!d) return;
    const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
    void popupMenu(rowMenu([d.id]), { x: box.left, y: box.bottom + 4 });
  }

  const multiTotal = $derived(ids.reduce((sum, id) => sum + (downloads.get(id)?.total ?? 0), 0));
</script>

{#snippet big(parts: Parts)}
  <span class="v">{parts.before}<b>{parts.value}</b>{parts.after}</span>
{/snippet}

<!-- Paths and links wrap between folders, never inside a name. -->
{#snippet breakable(value: string)}
  {#each pathPieces(value) as piece, i (i)}{piece}<wbr />{/each}
{/snippet}

{#snippet copyButton(key: string, text: string, label: string)}
  <IconButton label={copied === key ? t("details.copied") : label} onclick={() => copy(key, text)}>
    <Icon name={copied === key ? "check" : "copy"} />
  </IconButton>
{/snippet}

<aside class="pane" aria-label={t("details.label")}>
  {#if d}
    <header class="head">
      <div class="title-row">
        <span class="type"><Icon name={categoryIcon(name)} size={20} /></span>
        <div class="titles">
          <h2 class="name">{name}</h2>
          <p class="sub">
            {t("status.join", {
              first: hostOf(d.url),
              second:
                d.total !== null ? formatBytes(d.total) : t(`state.${d.state}` as PlainMessageKey),
            })}
          </p>
        </div>
        <IconButton label={t("details.close")} onclick={() => app.togglePane(false)}>
          <Icon name="close" />
        </IconButton>
      </div>
      <div class="actions">
        <Button
          variant={primary === "open" ? "primary" : "secondary"}
          onclick={() => app.run(primary, [d.id])}
        >
          {#snippet icon()}<Icon name={ACTION_ICONS[primary] ?? "play"} />{/snippet}
          {t(`action.${primary}` as PlainMessageKey)}
        </Button>
        <IconButton label={t("action.show")} onclick={() => app.run("show", [d.id])}>
          <Icon name="folder" />
        </IconButton>
        <IconButton label={t("action.more")} onclick={more}>
          <Icon name="more" />
        </IconButton>
      </div>
    </header>

    {#if needsAttention(d.state) && d.error}
      <section class="problem" class:error={d.state !== "needs_new_link"}>
        <span class="sign"><Icon name="alert" /></span>
        <div class="problem-text">
          <p class="what">{errorText(d.error)}</p>
          {#if d.state === "needs_new_link"}
            <form class="refresh" onsubmit={submitRefresh}>
              <label class="visually-hidden" for="refresh-url">{t("details.refresh.label")}</label>
              <input
                id="refresh-url"
                bind:this={refreshInput}
                bind:value={freshUrl}
                type="url"
                spellcheck="false"
                autocomplete="off"
                placeholder={t("details.refresh.placeholder")}
              />
              <Button type="submit" disabled={!freshUrl.trim()}
                >{t("details.refresh.submit")}</Button
              >
            </form>
            <p class="hint">{refreshError ?? t("details.refresh.hint")}</p>
          {/if}
          {#if d.error.message}
            <details class="technical">
              <summary>{t("details.technical")}</summary>
              <code>{d.error.message}</code>
            </details>
          {/if}
        </div>
      </section>
    {/if}

    <dl class="stats">
      {#if running}
        <div class="stat">
          <dt>{t("details.speed")}</dt>
          <dd>{@render big(speedParts(bps))}</dd>
        </div>
        <div class="stat">
          <dt>{t("details.received")}</dt>
          <dd>{@render big(progressParts(d.downloaded, d.total))}</dd>
        </div>
        <div class="stat">
          <dt>{t("details.left")}</dt>
          <dd><span class="v"><b>{eta === null ? "–" : formatDuration(eta)}</b></span></dd>
        </div>
        <div class="stat">
          <dt>{t("details.connections")}</dt>
          <dd><span class="v"><b>{d.connections}</b></span></dd>
        </div>
      {:else if complete}
        <div class="stat">
          <dt>{t("details.size")}</dt>
          <dd>{@render big(bytesParts(d.total ?? d.downloaded))}</dd>
        </div>
        <div class="stat">
          <dt>{t("details.finished")}</dt>
          <dd>
            <span class="v"
              ><b>{d.completedAt ? formatWhen(d.completedAt, clock.now) : "–"}</b></span
            >
          </dd>
        </div>
      {:else}
        <div class="stat">
          <dt>{t("details.received")}</dt>
          <dd>{@render big(progressParts(d.downloaded, d.total))}</dd>
        </div>
        {#if d.total}
          <div class="stat">
            <dt>{t("details.left_bytes")}</dt>
            <dd>{@render big(bytesParts(Math.max(0, d.total - d.downloaded)))}</dd>
          </div>
        {/if}
      {/if}
    </dl>

    <section class="block">
      <div class="label">
        <h3>{t("details.map")}</h3>
        <span class="note" class:ok={verified}>
          {#if verified}<Icon name="check" size={14} />{/if}{mapNote}
        </span>
      </div>
      {#if d.total}
        <FileMap
          segments={d.segments}
          total={d.total}
          downloaded={d.downloaded}
          {complete}
          live={running}
          label={t("details.map.label", { percent: t("percent", { value: pct ?? 0 }) })}
        />
      {:else}
        <p class="muted">{t("details.map.unknown")}</p>
      {/if}
    </section>

    {#if running}
      <section class="block">
        <div class="label">
          <h3>
            {samples.length >= SAMPLES ? t("details.throughput") : t("details.throughput.start")}
          </h3>
          {#if peak > 0}<span class="note">{t("details.peak", { speed: formatSpeed(peak) })}</span
            >{/if}
        </div>
        <Sparkline {samples} capacity={SAMPLES} label={t("details.throughput")} />
      </section>
      {#if workers.length}
        <section class="block">
          <div class="label"><h3>{t("details.connections")}</h3></div>
          <ul class="workers">
            {#each workers as w (w.id)}
              <li>
                <span class="wid">#{w.id}</span>
                <span class="wbar"
                  ><span style:transform="scaleX({w.size ? w.done / w.size : 0})"></span></span
                >
                <span class="wleft"
                  >{t("percent", { value: w.size ? Math.floor((w.done / w.size) * 100) : 0 })}</span
                >
              </li>
            {/each}
          </ul>
        </section>
      {/if}
    {/if}

    <section class="block">
      <div class="label"><h3>{t("details.file")}</h3></div>
      <dl class="facts">
        {#if d.path}
          <div class="fact">
            <dt>{complete ? t("details.saved") : t("details.saving")}</dt>
            <dd class="mono selectable path" title={d.path}>{@render breakable(d.path)}</dd>
            {@render copyButton("path", d.path, t("action.copy_value"))}
          </div>
        {/if}
        <div class="fact">
          <dt>{t("details.link")}</dt>
          <dd class="mono selectable path" title={d.url}>{@render breakable(d.url)}</dd>
          {@render copyButton("url", d.url, t("action.copy"))}
        </div>
        {#if refreshed}
          <div class="fact">
            <dt>{t("details.refreshed")}</dt>
            <dd class="mono selectable path" title={refreshed.url}>
              {@render breakable(refreshed.url)}
            </dd>
            {@render copyButton("fresh", refreshed.url, t("action.copy"))}
          </div>
        {/if}
        <div class="fact">
          <dt>{t("details.checksum")}</dt>
          <dd>
            <span class:ok={verified}>{checksumText}</span>
            {#if info?.checksum}
              <span class="mono small selectable">{info.checksum.expected}</span>
              {#if info.checksum.header}
                <span class="small"
                  >{t("details.checksum.from", { header: info.checksum.header })}</span
                >
              {/if}
            {/if}
          </dd>
          {#if info?.checksum}{@render copyButton(
              "sum",
              info.checksum.expected,
              t("action.copy_value"),
            )}{/if}
        </div>
        <div class="fact">
          <dt>{t("details.resume")}</dt>
          <dd>{d.resumable ? t("details.resumable") : t("details.not_resumable")}</dd>
        </div>
      </dl>
    </section>

    {#if !complete}
      <section class="block">
        <label class="label" for="speed-limit"><span>{t("details.limit")}</span></label>
        <div class="limit">
          <input
            id="speed-limit"
            bind:value={limitText}
            inputmode="decimal"
            placeholder={t("details.limit.none")}
            aria-label={t("details.limit.input")}
            aria-describedby="speed-limit-hint"
            onkeydown={(event) => {
              if (event.key === "Enter") applyLimit();
            }}
            onblur={applyLimit}
          />
          <span class="unit">{t("details.limit.unit")}</span>
        </div>
        <p class="hint" id="speed-limit-hint">{t("details.limit.hint")}</p>
      </section>
    {/if}
  {:else if ids.length > 1}
    <div class="none">
      <p class="none-title">{t("details.many", { count: ids.length })}</p>
      <p class="muted">{formatBytes(multiTotal)}</p>
    </div>
  {:else}
    <div class="none"><p class="muted">{t("details.none")}</p></div>
  {/if}
</aside>

<style>
  .pane {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
    height: 100%;
    padding: var(--space-4) var(--space-5) var(--space-8);
    overflow-y: auto;
    overflow-x: hidden;
  }

  .head {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .title-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
  }
  .type {
    display: inline-flex;
    margin-top: 2px;
    color: var(--color-text-subtle);
  }
  .titles {
    flex: 1;
    min-width: 0;
  }
  .name {
    margin: 0;
    font-size: var(--text-15);
    line-height: var(--leading-15);
    font-weight: var(--weight-title);
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    user-select: text;
    -webkit-user-select: text;
    cursor: text;
  }
  .sub {
    margin: 2px 0 0;
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    font-weight: var(--weight-secondary);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding-left: 32px;
  }

  .problem {
    display: flex;
    gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-floating);
  }
  .sign {
    display: inline-flex;
    margin-top: 1px;
    color: var(--color-warning);
  }
  .problem.error .sign {
    color: var(--color-error);
  }
  .problem-text {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    flex: 1;
    min-width: 0;
  }
  .what {
    margin: 0;
  }
  .refresh {
    display: flex;
    gap: var(--space-2);
  }
  .refresh input,
  .limit input {
    flex: 1;
    min-width: 0;
    height: var(--control-height);
    padding: 0 var(--space-2);
    border: 1px solid var(--color-hairline-strong);
    border-bottom-color: var(--color-control-stroke);
    border-radius: var(--radius-control);
    background: var(--color-field);
    color: var(--color-text);
    outline: none;
  }
  .refresh input {
    font-family: var(--font-mono);
    font-size: var(--text-12);
  }
  .refresh input::placeholder {
    font-family: var(--font-sans);
    font-size: var(--text-13);
  }
  .refresh input:focus,
  .limit input:focus {
    border-color: var(--color-focus);
    box-shadow: 0 0 0 1px var(--color-focus);
  }
  .technical summary {
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    cursor: default;
  }
  .technical code {
    display: block;
    margin-top: var(--space-1);
    font-family: var(--font-mono);
    font-size: var(--text-12);
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
    user-select: text;
    -webkit-user-select: text;
  }

  /* The hero card's stats: large tabular numbers, small units. */
  .stats {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-4) var(--space-4);
    margin: 0;
    padding: var(--space-4) 0;
    border-block: 1px solid var(--color-hairline);
  }
  .stat {
    min-width: 0;
  }
  .stat dt {
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    font-weight: var(--weight-secondary);
  }
  .stat dd {
    margin: var(--space-1) 0 0;
  }
  .v {
    color: var(--color-text-muted);
    white-space: nowrap;
  }
  .v b {
    color: var(--color-text);
    font-size: var(--text-20);
    line-height: var(--leading-20);
    font-weight: var(--weight-medium);
    letter-spacing: -0.02em;
  }

  .block {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .label {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-3);
    color: var(--color-text-muted);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    font-weight: var(--weight-medium);
  }
  /* Section titles are headings, so a screen reader can jump between them. */
  .label h3 {
    margin: 0;
    font: inherit;
  }
  .note {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--color-text-subtle);
    font-weight: var(--weight-secondary);
  }
  .ok {
    color: var(--color-success);
  }

  .workers {
    display: grid;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }
  .workers li {
    display: grid;
    grid-template-columns: 28px minmax(0, 1fr) 40px;
    align-items: center;
    gap: var(--space-2);
  }
  .wid,
  .wleft {
    color: var(--color-text-subtle);
  }
  .wleft {
    text-align: right;
  }
  .wbar {
    position: relative;
    height: 4px;
    overflow: hidden;
    background: var(--color-progress-track);
  }
  .wbar span {
    position: absolute;
    inset: 0;
    transform-origin: left center;
    background: var(--gradient-progress);
    transition: transform var(--duration-tick) var(--ease-data);
  }

  .facts {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin: 0;
  }
  .fact {
    display: grid;
    grid-template-columns: 76px minmax(0, 1fr) auto;
    align-items: start;
    gap: var(--space-2);
  }
  .fact dt {
    padding-top: 5px;
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }
  .fact dd {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding-top: 4px;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .fact :global(.icon-button) {
    margin-top: -2px;
  }
  .fact dd.path {
    overflow-wrap: break-word;
  }
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }
  .small {
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }
  .selectable {
    user-select: text;
    -webkit-user-select: text;
    cursor: text;
  }

  .limit {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    max-width: 200px;
  }
  .unit {
    color: var(--color-text-subtle);
  }
  .hint,
  .muted {
    margin: 0;
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }

  .none {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    height: 100%;
    text-align: center;
  }
  .none p {
    margin: 0;
  }
  .none-title {
    font-weight: var(--weight-medium);
  }
</style>
