<script lang="ts" module>
  /** `sha256` as people write it: `SHA-256`. */
  function algorithmName(algorithm: string): string {
    return algorithm.toUpperCase().replace(/^SHA(\d)/, "SHA-$1");
  }
</script>

<!--
  The add sheet (APP-DESIGN #6 to #8): one sheet inside the window for every
  way a link arrives. It probes the link at once and shows what it finds
  (name, size, type, host, resume, a server checksum), warns before a bad
  download rather than after, and asks for one decision: Enter downloads.
  The file name and the folder are the visible fields; connections, the
  speed limit, the queue, a start time, the checksum and "Add paused" wait
  under "More options". A download waits in its queue for its turn (the
  queue of the view the sheet was opened from, else Main), or for the time
  chosen.
  Several links become a checklist with one folder for all.

  A native modal <dialog>: it traps focus and Esc closes it.
-->
<script lang="ts">
  import Button from "@boltget/ui/components/Button.svelte";
  import Checkbox from "@boltget/ui/components/Checkbox.svelte";
  import IconButton from "@boltget/ui/components/IconButton.svelte";
  import Select from "@boltget/ui/components/Select.svelte";
  import TextField from "@boltget/ui/components/TextField.svelte";
  import { t } from "@boltget/ui/messages";
  import { untrack } from "svelte";
  import { app } from "../app.svelte";
  import type { AddDefaults, DestinationCheck, DownloadId } from "../bindings";
  import { downloads, errorText } from "../engine";
  import { displayName, formatBytes, formatWhen, hostOf, splitFolder } from "../format";
  import Icon from "../Icon.svelte";
  import { CATEGORY_LABELS, categoryIcon, categoryOf } from "../list/category";
  import { queueOfView } from "../list/model";
  import { dateTimeValue, nextHour, parseDateTime } from "../list/queue";
  import { folderFor } from "../settings/model";
  import { popupMenu, type MenuEntry } from "../menu";
  import { inTauri } from "../native";
  import {
    findDuplicate,
    parseChecksum,
    parseConnections,
    parseFileName,
    parseLimit,
    readLinks,
    type AddProblem,
  } from "./links";
  import { ProbeQueue, problemOf, type ProbeEntry } from "./probe";

  /** A probe shows its skeleton only when it takes longer than this. */
  const SKELETON_MS = 150;
  const EXIT_MS = 150;
  const MORE_KEY = "boltget.add.more";

  let dialog: HTMLDialogElement | undefined = $state();
  let field: HTMLTextAreaElement | undefined = $state();
  let form: HTMLFormElement | undefined = $state();

  let open = $state(false);
  let closing = $state(false);
  let lastKey = -1;

  let text = $state("");
  let queue: ProbeQueue | null = null;
  /** Bumped when the queue changes, so probe-derived values recompute. */
  let version = $state(0);

  let defaults = $state<AddDefaults>({ downloadsFolder: null, recentFolders: [] });
  /** A folder the user chose; `null` while the sheet picks one by category. */
  let folder = $state<string | null>(null);

  let name = $state("");
  let nameEdited = $state(false);
  let namedFor = "";

  let more = $state(readMore());
  let connections = $state("");
  let limit = $state("");
  let checksum = $state("");
  let paused = $state(false);
  /** The queue it waits in, and whether it waits for its turn or a time. */
  let queueChoice = $state("1");
  let startMode = $state<"turn" | "later">("turn");
  let startAt = $state("");
  let laterProblem = $state<string | null>(null);
  /** Links of a list the user unticked. */
  let excluded = $state.raw<ReadonlySet<string>>(new Set());

  let problems = $state<Partial<Record<"name" | "connections" | "limit" | "checksum", AddProblem>>>(
    {},
  );
  let submitError = $state<string | null>(null);
  let submitting = $state(false);
  let dest = $state<DestinationCheck | null>(null);
  let skeleton = $state(false);

  const reduced =
    typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;

  function readMore(): boolean {
    try {
      return localStorage.getItem(MORE_KEY) === "open";
    } catch {
      return false;
    }
  }

  function toggleMore() {
    more = !more;
    try {
      localStorage.setItem(MORE_KEY, more ? "open" : "closed");
    } catch {
      // A convenience only.
    }
  }

  // Derived state ---------------------------------------------------------

  const input = $derived(readLinks(text));
  const urls = $derived(input.kind === "links" ? input.urls : []);
  const single = $derived(urls.length === 1 ? (urls[0] ?? null) : null);
  const many = $derived(urls.length > 1);

  function entryOf(url: string): ProbeEntry | undefined {
    void version;
    return queue?.get(url);
  }

  const entry = $derived(single ? entryOf(single) : undefined);
  const preview = $derived(entry?.status === "ready" ? entry.preview : null);
  const failure = $derived(entry?.status === "failed" ? entry.error : null);

  const selected = $derived(urls.filter((u) => !excluded.has(u)));
  const targets = $derived(many ? selected : single ? [single] : []);

  /**
   * The folder the sheet picks (APP-DESIGN #6): the category's folder for
   * the file's type, else the default download folder. A list goes to the
   * category folder its files share, else the default.
   */
  const automatic = $derived.by((): { path: string | null; category: string | null } => {
    const settings = downloads.settings;
    const names = many
      ? selected.map((u) => entryName(u))
      : single && (preview || name)
        ? [shownName]
        : [];
    const paths = new Set(names.map((n) => folderFor(n, settings, defaults.downloadsFolder)));
    const [only] = paths;
    if (paths.size !== 1 || !only || only === defaults.downloadsFolder) {
      return { path: defaults.downloadsFolder, category: null };
    }
    const category = categoryOf(names[0] ?? "");
    return { path: only, category: t(CATEGORY_LABELS[category]) };
  });
  const folderPath = $derived(folder ?? automatic.path);
  const folderCategory = $derived(folder === null ? automatic.category : null);
  const folderParts = $derived(
    folderPath ? splitFolder(folderPath) : { parent: "", leaf: t("add.folder.downloads") },
  );

  const listed = $derived(
    downloads.downloads
      .filter((d) => !app.hidden.has(d.id))
      .map((d) => ({ id: d.id, url: d.url, state: d.state, completedAt: d.completedAt })),
  );
  const duplicate = $derived(single ? findDuplicate(single, listed) : null);

  const shownName = $derived(
    name || (preview?.fileName ?? (single ? displayName(null, single) : "")),
  );

  /** The one notice under the preview, most important first. */
  const notice = $derived.by(
    (): { text: string; tone: "warning" | "info"; action?: "open" | "show" } | null => {
      if (!single || failure) return null;
      if (preview?.webPage) return { text: t("add.warn.web_page"), tone: "warning" };
      if (preview?.size && dest?.freeBytes != null && preview.size > dest.freeBytes) {
        return {
          text: t("add.warn.space", { free: formatBytes(dest.freeBytes) }),
          tone: "warning",
        };
      }
      if (duplicate?.kind === "downloaded") {
        const at = duplicate.completedAt;
        const today = at !== null && new Date(at).toDateString() === new Date().toDateString();
        return {
          text:
            at === null
              ? t("add.warn.listed")
              : today
                ? t("add.warn.downloaded_today", { time: formatWhen(at) })
                : t("add.warn.downloaded", { date: formatWhen(at) }),
          tone: "info",
          action: "open",
        };
      }
      if (duplicate?.kind === "listed") {
        return { text: t("add.warn.listed"), tone: "info", action: "show" };
      }
      if (dest?.renamed && preview) {
        return {
          text: t("add.warn.renamed", { name: shownName, saved: dest.fileName }),
          tone: "info",
        };
      }
      return null;
    },
  );

  const queueOptions = $derived(
    downloads.queues.map((q) => ({ value: String(q.id), label: app.queueName(q) })),
  );
  const later = $derived(startMode === "later" && !paused);

  const submitLabel = $derived.by(() => {
    if (many) {
      const count = selected.length;
      if (later) return t("add.submit.many_later", { count });
      return paused ? t("add.submit.many_paused", { count }) : t("add.submit.many", { count });
    }
    if (paused) return t("add.submit.paused");
    if (later) return t("add.submit.later");
    return duplicate?.kind === "downloaded" ? t("add.submit.again") : t("add.submit");
  });

  // Opening and closing ---------------------------------------------------

  $effect(() => {
    const request = app.adding;
    untrack(() => {
      if (request && !open) show(request.text, request.key);
      else if (request && open && request.key !== lastKey) merge(request.text, request.key);
      else if (!request && open) hide();
    });
  });

  function show(initial: string, key: number) {
    lastKey = key;
    open = true;
    closing = false;
    text = initial;
    name = "";
    nameEdited = false;
    namedFor = "";
    connections = "";
    limit = "";
    checksum = "";
    paused = false;
    const viewQueue = queueOfView(app.view);
    queueChoice = String(
      viewQueue !== null && downloads.queue(viewQueue) ? viewQueue : (downloads.queues[0]?.id ?? 1),
    );
    startMode = "turn";
    startAt = dateTimeValue(nextHour(Date.now()));
    laterProblem = null;
    excluded = new Set();
    problems = {};
    submitError = null;
    dest = null;
    folder = null;
    queue = new ProbeQueue({
      probe: (url) => downloads.probe(url),
      onChange: () => (version += 1),
    });
    queue.want(readLinks(initial).kind === "links" ? linksOf(initial) : [], true);
    void downloads.addDefaults().then((result) => {
      if (result.status !== "ok" || !open) return;
      defaults = result.data;
    });
    queueMicrotask(() => {
      dialog?.showModal();
      field?.focus();
      field?.setSelectionRange(field.value.length, field.value.length);
    });
  }

  function linksOf(value: string): string[] {
    const read = readLinks(value);
    return read.kind === "links" ? read.urls : [];
  }

  /** More links arrived (a drop, Ctrl+V outside the field) while open. */
  function merge(more: string, key: number) {
    lastKey = key;
    const known = new Set(urls);
    const fresh = linksOf(more).filter((u) => !known.has(u));
    if (fresh.length === 0) return;
    text = [text.trimEnd(), ...fresh].filter(Boolean).join("\n");
    queue?.want(linksOf(text), true);
    field?.focus();
  }

  function hide() {
    queue?.dispose();
    queue = null;
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
    // Esc fires `cancel`; the sheet animates out instead of vanishing.
    event?.preventDefault();
    app.closeAdd();
  }

  // Probing ---------------------------------------------------------------

  function oninput(event: Event) {
    const kind = (event as InputEvent).inputType ?? "";
    const now = kind === "insertFromPaste" || kind === "insertFromDrop";
    // The binding may update `text` after this handler; read the field.
    const value = (event.currentTarget as HTMLTextAreaElement).value;
    queue?.want(linksOf(value), now);
    submitError = null;
  }

  // The skeleton waits 150 ms, so a fast probe never flashes one.
  $effect(() => {
    if (entry?.status !== "probing") {
      skeleton = false;
      return;
    }
    const wait = Math.max(0, SKELETON_MS - (Date.now() - entry.since));
    const timer = setTimeout(() => (skeleton = true), wait);
    return () => clearTimeout(timer);
  });

  // The name follows the probe until the user edits it.
  $effect(() => {
    const url = single;
    const resolved = preview?.fileName;
    untrack(() => {
      if (url !== namedFor) {
        namedFor = url ?? "";
        nameEdited = false;
        name = "";
        problems = { ...problems, name: undefined };
      }
      if (!nameEdited && resolved) name = resolved;
    });
  });

  // Where it would land: the final name (with " (1)" if taken) and the space.
  $effect(() => {
    const target = preview ? name.trim() : "";
    const dir = folderPath;
    if (!target || parseFileName(target).ok === false) {
      dest = null;
      return;
    }
    const timer = setTimeout(() => {
      void downloads.checkDestination(dir, target).then((result) => {
        if (result.status === "ok" && name.trim() === target && folderPath === dir) {
          dest = result.data;
        }
      });
    }, 120);
    return () => clearTimeout(timer);
  });

  // Folder ----------------------------------------------------------------

  async function choose() {
    const result = await downloads.chooseFolder(folderPath);
    if (result.status === "ok" && result.data) folder = result.data;
    field?.focus();
  }

  function folders(): string[] {
    const all = [...defaults.recentFolders];
    if (defaults.downloadsFolder) all.push(defaults.downloadsFolder);
    if (folderPath) all.unshift(folderPath);
    return [...new Map(all.map((f) => [f.toLowerCase(), f])).values()].slice(0, 4);
  }

  function folderMenu(event: MouseEvent) {
    if (!inTauri) {
      void choose();
      return;
    }
    const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
    const entries: MenuEntry[] = folders().map((f) => ({
      text: f,
      checked: f.toLowerCase() === folderPath?.toLowerCase(),
      run: () => (folder = f),
    }));
    entries.push("separator", { text: t("add.folder.choose"), run: () => void choose() });
    void popupMenu(entries, { x: box.left, y: box.bottom + 4 });
  }

  // Submitting ------------------------------------------------------------

  function toggleLink(url: string, on: boolean) {
    // eslint-disable-next-line svelte/prefer-svelte-reactivity -- replaced whole
    const next = new Set(excluded);
    if (on) next.delete(url);
    else next.add(url);
    excluded = next;
  }

  function toggleAll(on: boolean) {
    excluded = on ? new Set() : new Set(urls);
  }

  function onfieldkey(event: KeyboardEvent) {
    // Enter downloads; Shift+Enter starts a new line for another link.
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      form?.requestSubmit();
    }
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault();
    if (submitting || targets.length === 0) return;
    const c = parseConnections(connections);
    const l = parseLimit(limit);
    const k = many ? ({ ok: true, value: null } as const) : parseChecksum(checksum);
    const n = single && name.trim() ? parseFileName(name) : ({ ok: true, value: "" } as const);
    problems = {
      connections: c.ok ? undefined : c.problem,
      limit: l.ok ? undefined : l.problem,
      checksum: k.ok ? undefined : k.problem,
      name: n.ok ? undefined : n.problem,
    };
    const at = later ? parseDateTime(startAt) : null;
    laterProblem = later && (at === null || at <= Date.now()) ? t("later.past") : null;
    if (!c.ok || !l.ok || !k.ok || !n.ok || laterProblem) {
      if (!c.ok || !l.ok || !k.ok || laterProblem) more = true;
      return;
    }
    // A name the user typed travels; the probed one the engine finds again.
    const fileName = single && nameEdited && n.value ? n.value : undefined;
    submitting = true;
    submitError = null;
    const added: DownloadId[] = [];
    let addedName = "";
    for (const url of targets) {
      const result = await downloads.add({
        url,
        directory: folderPath ?? undefined,
        fileName,
        connections: c.value ?? undefined,
        speedLimit: l.value ?? undefined,
        checksum: k.value ?? undefined,
        paused,
        queue: Number(queueChoice),
        startAt: at ?? undefined,
      });
      if (result.status === "error") {
        submitError = errorText(result.error);
        break;
      }
      added.push(result.data.id);
      addedName ||= fileName ?? entryName(url);
    }
    submitting = false;
    if (added.length) {
      app.select(added);
      app.announce(t("add.announce", { count: added.length, name: addedName }));
    }
    if (submitError === null) app.closeAdd();
    else if (added.length) {
      // Keep only what did not go in, so a second Enter does not repeat.
      const done = new Set(targets.slice(0, added.length));
      text = urls.filter((u) => !done.has(u)).join("\n");
    }
  }

  function entryName(url: string): string {
    const e = entryOf(url);
    return e?.status === "ready" ? e.preview.fileName : displayName(null, url);
  }

  function showExisting() {
    if (!duplicate) return;
    if (notice?.action === "open") void downloads.open(duplicate.id);
    else {
      app.select([duplicate.id]);
      app.togglePane(true);
      app.closeAdd();
    }
  }
</script>

{#snippet meta(parts: string[])}
  {#each parts as part, i (i)}{#if i}<span class="dot" aria-hidden="true">·</span>{/if}<span
      >{part}</span
    >{/each}
{/snippet}

<dialog
  bind:this={dialog}
  class="sheet"
  class:closing
  class:reduced
  aria-labelledby="add-title"
  oncancel={cancel}
  onclick={(event) => {
    // A click on the backdrop (the dialog element itself) closes.
    if (event.target === dialog) cancel();
  }}
>
  {#if open}
    <form bind:this={form} class="body" onsubmit={submit} novalidate>
      <header class="head">
        <h2 id="add-title" class="title">{t("add.title")}</h2>
        <IconButton label={t("action.cancel")} onclick={() => cancel()}>
          <Icon name="close" />
        </IconButton>
      </header>

      <label class="visually-hidden" for="add-link">{t("add.link")}</label>
      <textarea
        id="add-link"
        bind:this={field}
        bind:value={text}
        class="link"
        class:list={many}
        rows={many ? Math.min(4, urls.length) : 1}
        spellcheck="false"
        autocomplete="off"
        placeholder={t("add.link.placeholder")}
        aria-invalid={input.kind === "invalid"}
        aria-describedby="add-status"
        {oninput}
        onkeydown={onfieldkey}></textarea>

      <div id="add-status" class="status" aria-live="polite">
        {#if input.kind === "invalid"}
          <p class="line muted">{t("add.invalid")}</p>
        {:else if single}
          <!-- One link: what the probe found. -->
          <div class="card" class:quiet={!preview && !failure}>
            {#if failure}
              {@const problem = problemOf(failure)}
              <span class="type warn"><Icon name="alert" size={20} /></span>
              <div class="card-text">
                <p class="problem">
                  {problem === "unreachable"
                    ? t("add.error.unreachable")
                    : problem === "auth"
                      ? t("add.error.auth")
                      : errorText(failure)}
                </p>
                <p class="meta">{hostOf(single)}</p>
              </div>
              {#if problem === "unreachable"}
                <Button variant="ghost" onclick={() => single && queue?.retry(single)}
                  >{t("action.retry")}</Button
                >
              {/if}
            {:else if preview}
              <span class="type" class:warn={preview.webPage}>
                <Icon name={preview.webPage ? "globe" : categoryIcon(shownName)} size={20} />
              </span>
              <div class="card-text">
                <label class="visually-hidden" for="add-name">{t("add.name")}</label>
                <input
                  id="add-name"
                  class="name"
                  bind:value={name}
                  oninput={() => {
                    nameEdited = true;
                    problems = { ...problems, name: undefined };
                  }}
                  spellcheck="false"
                  autocomplete="off"
                  aria-invalid={!!problems.name}
                  title={shownName}
                />
                <p class="meta">
                  {@render meta([
                    preview.size !== null ? formatBytes(preview.size) : t("add.meta.unknown_size"),
                    hostOf(single),
                    preview.resumable ? t("add.meta.resume") : t("add.meta.no_resume"),
                    ...(preview.checksum
                      ? [
                          t("add.meta.checksum", {
                            algorithm: algorithmName(preview.checksum.algorithm),
                          }),
                        ]
                      : []),
                  ])}
                </p>
              </div>
            {:else}
              <span class="type"><Icon name={categoryIcon(shownName)} size={20} /></span>
              <div class="card-text">
                <p class="name static">{shownName}</p>
                <p class="meta">
                  {#if skeleton}<span class="probing">{t("add.probing")}</span>{:else}{hostOf(
                      single,
                    )}{/if}
                </p>
              </div>
            {/if}
          </div>
          {#if problems.name}
            <p class="line error">{t(problems.name)}</p>
          {:else if notice}
            <p class="line {notice.tone}">
              <Icon name={notice.tone === "warning" ? "alert" : "link"} size={14} />
              <span>{notice.text}</span>
              {#if notice.action}
                <button type="button" class="inline" onclick={showExisting}
                  >{notice.action === "open" ? t("action.open") : t("add.show_existing")}</button
                >
              {/if}
            </p>
          {/if}
        {:else if many}
          <!-- Several links: a checklist, probed a few at a time (#8). -->
          <div class="list">
            <div class="list-head">
              <Checkbox
                checked={selected.length === urls.length}
                indeterminate={selected.length > 0 && selected.length < urls.length}
                onchange={(event) => toggleAll(event.currentTarget.checked)}
              >
                {t("add.list.all")}
              </Checkbox>
              <span class="muted">
                {selected.length === urls.length
                  ? t("add.list.count", { count: urls.length })
                  : t("add.list.selected", { selected: selected.length, count: urls.length })}
              </span>
            </div>
            <ul class="items">
              {#each urls as url (url)}
                {@const e = entryOf(url)}
                {@const itemName =
                  e?.status === "ready" ? e.preview.fileName : displayName(null, url)}
                <li class:off={excluded.has(url)}>
                  <Checkbox
                    checked={!excluded.has(url)}
                    label={itemName}
                    onchange={(event) => toggleLink(url, event.currentTarget.checked)}
                  />
                  <span class="item-icon"><Icon name={categoryIcon(itemName)} /></span>
                  <span class="item-name" title={url}>{itemName}</span>
                  <span class="item-meta">
                    {#if e?.status === "ready"}
                      {#if e.preview.webPage}
                        <span class="warn-text">{t("add.list.web_page")}</span>
                      {:else}
                        {e.preview.size !== null ? formatBytes(e.preview.size) : "–"}
                      {/if}
                    {:else if e?.status === "failed"}
                      <span class="warn-text" title={errorText(e.error)}
                        ><Icon name="alert" size={14} /></span
                      >
                    {:else}
                      <span class="pending" aria-label={t("add.probing")}></span>
                    {/if}
                  </span>
                </li>
              {/each}
            </ul>
          </div>
        {/if}
      </div>

      <div class="row folder-row">
        <span class="label">{t("add.folder")}</span>
        <button
          type="button"
          class="folder"
          title={folderPath ?? ""}
          aria-label={t("add.folder.label", { folder: folderPath ?? folderParts.leaf })}
          onclick={folderMenu}
        >
          <Icon name="folder" />
          <span class="folder-path"
            ><span class="parent"><span dir="ltr">{folderParts.parent}</span></span><span
              class="leaf">{folderParts.leaf}</span
            ></span
          >
          <Icon name="down" size={14} />
        </button>
      </div>
      {#if folderCategory}
        <p class="line muted indent">{t("add.folder.category", { category: folderCategory })}</p>
      {/if}
      {#if dest && !dest.folderExists}
        <p class="line muted indent">{t("add.folder.new")}</p>
      {/if}

      <button
        type="button"
        class="disclosure"
        aria-expanded={more}
        aria-controls="add-more"
        onclick={toggleMore}
      >
        <span class="chev" class:open={more}><Icon name="chevron" size={14} /></span>
        {t("add.more")}
      </button>
      {#if more}
        <div id="add-more" class="more">
          <div class="field">
            <label for="add-connections">{t("add.connections")}</label>
            <input
              id="add-connections"
              bind:value={connections}
              inputmode="numeric"
              placeholder={t("add.connections.auto")}
              aria-invalid={!!problems.connections}
              oninput={() => (problems = { ...problems, connections: undefined })}
            />
          </div>
          <div class="field">
            <label for="add-limit">{t("add.limit")}</label>
            <div class="with-unit">
              <input
                id="add-limit"
                bind:value={limit}
                inputmode="decimal"
                placeholder={t("add.limit.none")}
                aria-invalid={!!problems.limit}
                oninput={() => (problems = { ...problems, limit: undefined })}
              />
              <span class="unit">{t("add.limit.unit")}</span>
            </div>
          </div>
          <div class="field">
            <label for="add-queue">{t("add.queue")}</label>
            <Select id="add-queue" bind:value={queueChoice} options={queueOptions} />
          </div>
          <div class="field">
            <label for="add-start">{t("add.start")}</label>
            <Select
              id="add-start"
              bind:value={startMode}
              disabled={paused}
              options={[
                { value: "turn", label: t("add.start.turn") },
                { value: "later", label: t("add.start.later") },
              ]}
            />
          </div>
          {#if later}
            <div class="field wide">
              <label for="add-start-at">{t("add.start.at")}</label>
              <TextField
                id="add-start-at"
                type="datetime-local"
                bind:value={startAt}
                invalid={!!laterProblem}
                oninput={() => (laterProblem = null)}
              />
            </div>
          {/if}
          {#if laterProblem}
            <p class="line error wide" role="alert">{laterProblem}</p>
          {/if}
          {#if !many}
            <div class="field wide">
              <label for="add-checksum">{t("add.checksum")}</label>
              <input
                id="add-checksum"
                class="mono"
                bind:value={checksum}
                spellcheck="false"
                autocomplete="off"
                placeholder={t("add.checksum.placeholder")}
                aria-invalid={!!problems.checksum}
                oninput={() => (problems = { ...problems, checksum: undefined })}
              />
            </div>
          {/if}
          {#if problems.connections || problems.limit || problems.checksum}
            <p class="line error wide">
              {t((problems.connections ?? problems.limit ?? problems.checksum) as AddProblem)}
            </p>
          {:else if preview?.checksum && !checksum.trim() && !many}
            <p class="line muted wide">
              {t("add.checksum.server", { algorithm: algorithmName(preview.checksum.algorithm) })}
            </p>
          {/if}
          <div class="wide">
            <Checkbox bind:checked={paused}>{t("add.paused")}</Checkbox>
          </div>
        </div>
      {/if}

      <footer class="foot">
        <p class="foot-note" class:error={!!submitError} aria-live="polite">
          {submitError ?? ""}
        </p>
        <Button onclick={() => cancel()}>{t("action.cancel")}</Button>
        <Button variant="primary" type="submit" disabled={targets.length === 0 || submitting}>
          {submitLabel}
        </Button>
      </footer>
    </form>
  {/if}
</dialog>

<style>
  .sheet {
    width: min(560px, calc(100vw - 48px));
    max-height: calc(100vh - 96px);
    margin: 72px auto auto;
    padding: 0;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-floating);
    background: var(--color-raised);
    color: var(--color-text);
    box-shadow: var(--elevation-floating);
    overflow-y: auto;
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
    font-size: var(--text-15);
    line-height: var(--leading-15);
    font-weight: var(--weight-title);
  }

  /* The link: Mono, one line until there are several. */
  .link {
    display: block;
    width: 100%;
    min-height: var(--control-height-lg);
    max-height: 120px;
    padding: 9px var(--space-3);
    border: 1px solid var(--color-hairline-strong);
    border-radius: var(--radius-control);
    background: var(--color-field);
    font-family: var(--font-mono);
    font-size: var(--text-12);
    line-height: 18px;
    resize: none;
    overflow-y: auto;
    white-space: nowrap;
    outline: none;
  }
  .link.list {
    white-space: pre;
  }
  .link::placeholder {
    font-family: var(--font-sans);
    font-size: var(--text-13);
    color: var(--color-text-subtle);
  }
  .link:focus,
  .more input:focus {
    border-color: var(--color-focus);
    box-shadow: 0 0 0 1px var(--color-focus);
  }

  .status {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .status:empty {
    display: none;
  }

  /* What the probe found: the website's hero card, small. */
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
  .type {
    display: inline-grid;
    place-items: center;
    flex: none;
    width: 36px;
    height: 36px;
    border-radius: var(--radius-control);
    background: var(--color-accent-tint);
    color: var(--color-accent);
  }
  .card.quiet .type {
    background: var(--color-overlay-hover);
    color: var(--color-text-subtle);
  }
  .type.warn {
    background: var(--color-overlay-hover);
    color: var(--color-warning);
  }
  .card-text {
    flex: 1;
    min-width: 0;
  }
  .name {
    display: block;
    width: 100%;
    margin: 0 0 0 calc(-1 * var(--space-1));
    padding: 1px var(--space-1);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    background: transparent;
    font-size: var(--text-13);
    line-height: var(--leading-13);
    font-weight: var(--weight-medium);
    text-overflow: ellipsis;
    outline: none;
  }
  .name:hover {
    border-color: var(--color-hairline);
  }
  .name:focus {
    border-color: var(--color-focus);
    background: var(--color-field);
  }
  .name.static {
    overflow: hidden;
    white-space: nowrap;
    color: var(--color-text-muted);
  }
  .name[aria-invalid="true"] {
    border-color: var(--color-error);
  }
  .meta {
    margin: 2px 0 0;
    color: var(--color-text-subtle);
    font-size: var(--text-12);
    line-height: var(--leading-12);
    font-weight: var(--weight-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dot {
    margin: 0 6px;
  }
  .problem {
    margin: 0;
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
    margin-top: 0;
    flex: none;
  }
  .line.warning :global(.icon) {
    color: var(--color-warning);
  }
  .line.info :global(.icon) {
    color: var(--color-text-subtle);
  }
  .line.error {
    color: var(--color-error);
  }
  .line.muted,
  .muted {
    color: var(--color-text-subtle);
  }
  .indent {
    padding-left: 72px;
  }
  .inline {
    flex: none;
    margin-left: auto;
    padding: 0;
    border: 0;
    background: none;
    color: var(--color-accent);
    font-weight: var(--weight-medium);
  }
  .inline:hover {
    text-decoration: underline;
  }

  /* Several links. */
  .list {
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-floating);
    background: var(--color-content);
  }
  .list-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--color-hairline);
    font-size: var(--text-12);
  }
  .items {
    max-height: 216px;
    margin: 0;
    padding: var(--space-1) 0;
    overflow-y: auto;
    list-style: none;
  }
  .items li {
    display: grid;
    grid-template-columns: auto auto minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--space-2);
    height: 32px;
    padding: 0 var(--space-3);
  }
  .items li.off .item-name,
  .items li.off .item-icon,
  .items li.off .item-meta {
    opacity: 0.45;
  }
  .item-icon {
    color: var(--color-text-subtle);
  }
  .item-name {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .item-meta {
    color: var(--color-text-subtle);
    font-size: var(--text-12);
  }
  .warn-text {
    display: inline-flex;
    color: var(--color-warning);
  }
  .pending {
    display: inline-block;
    width: 40px;
    height: 6px;
    border-radius: 3px;
    background: var(--color-overlay-press);
    animation: breathe 1.4s ease-in-out infinite;
  }

  /* Folder. */
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .label {
    flex: none;
    width: 60px;
    color: var(--color-text-muted);
    font-size: var(--text-12);
  }
  .folder {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: 1;
    min-width: 0;
    height: var(--control-height);
    padding: 0 var(--space-2);
    border: 1px solid var(--color-hairline-strong);
    border-radius: var(--radius-control);
    background: var(--color-raised);
    color: var(--color-text-subtle);
    text-align: left;
  }
  .folder:hover {
    background: var(--color-overlay-hover);
  }
  .folder-path {
    display: flex;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
  }
  .parent {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    /* Clips the start of a long path, so the part next to the name shows. */
    direction: rtl;
    text-align: left;
  }
  .parent > span {
    unicode-bidi: isolate;
    color: var(--color-text-subtle);
  }
  .leaf {
    flex: none;
    color: var(--color-text);
  }

  .disclosure {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    align-self: flex-start;
    margin-left: calc(-1 * var(--space-1));
    padding: 2px var(--space-1);
    border: 0;
    border-radius: var(--radius-control);
    background: none;
    color: var(--color-text-muted);
    font-size: var(--text-12);
  }
  .disclosure:hover {
    color: var(--color-text);
  }
  .chev {
    display: inline-flex;
    transition: transform var(--duration-small) var(--ease-enter);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .reduced .chev {
    transition: none;
  }

  .more {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
    padding: var(--space-3);
    border-radius: var(--radius-floating);
    background: var(--color-content);
    border: 1px solid var(--color-hairline);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }
  .field label {
    color: var(--color-text-muted);
    font-size: var(--text-12);
    line-height: var(--leading-12);
  }
  .wide {
    grid-column: 1 / -1;
  }
  .more input {
    width: 100%;
    min-width: 0;
    height: var(--control-height);
    padding: 0 var(--space-2);
    border: 1px solid var(--color-hairline-strong);
    border-radius: var(--radius-control);
    background: var(--color-field);
    outline: none;
  }
  .more input[aria-invalid="true"] {
    border-color: var(--color-error);
  }
  .more input.mono {
    font-family: var(--font-mono);
    font-size: var(--text-12);
  }
  .more input.mono::placeholder {
    font-family: var(--font-sans);
    font-size: var(--text-13);
  }
  .with-unit {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .unit {
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
    .pending {
      animation: none;
    }
  }
</style>
