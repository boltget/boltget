// The page's copy of the engine (docs/APP-DESIGN.md, "The engine bridge as
// built"). It loads the snapshot the backend sends on subscribe, applies the
// coalesced batches that follow, and exposes typed actions that update rows
// optimistically.
//
// Reactivity is fine-grained so 500 rows stay smooth: each download is a
// `Download` with its own `$state` fields, so a progress tick re-renders only
// the rows whose numbers changed. The list itself (`ids`) changes only when
// downloads are added or removed.
import type {
  AddDownload,
  AppSettings,
  DownloadDetails,
  DownloadError,
  DownloadEvent,
  DownloadId,
  DownloadRow,
  DownloadState,
  EngineMessage,
  EngineStatus,
  LimitScope,
  ProbePreview,
  ProgressItem,
  SegmentSpan,
  UiError,
  commands,
} from "../bindings";

/** The result shape of every generated command that can fail. */
export type Result<T> = { status: "ok"; data: T } | { status: "error"; error: UiError };

/** What the store needs from the backend; tests pass a fake. */
export interface EngineApi {
  /** Starts the stream; `onMessage` first receives a snapshot. */
  subscribe(onMessage: (message: EngineMessage) => void): Promise<void>;
  commands: Pick<
    typeof commands,
    | "addDownload"
    | "probeUrl"
    | "pauseDownload"
    | "resumeDownload"
    | "removeDownload"
    | "refreshLink"
    | "setSpeedLimit"
    | "updateSettings"
    | "setFocusedDownload"
    | "setUiVisible"
    | "downloadDetails"
    | "openDownload"
    | "showDownloadInFolder"
  >;
}

/** One download, with a reactive field per value the row shows. */
export class Download {
  readonly id: DownloadId;
  state = $state<DownloadState>("queued");
  url = $state("");
  fileName = $state<string | null>(null);
  path = $state<string | null>(null);
  total = $state<number | null>(null);
  downloaded = $state(0);
  bps = $state(0);
  connections = $state(0);
  resumable = $state(false);
  speedLimit = $state<number | null>(null);
  error = $state<DownloadError | null>(null);
  createdAt = $state(0);
  completedAt = $state<number | null>(null);
  /** The file map, while this download is focused and running. */
  segments = $state.raw<SegmentSpan[] | null>(null);

  constructor(row: DownloadRow) {
    this.id = row.id;
    this.update(row);
  }

  /** Takes every value of a row from the backend. */
  update(row: DownloadRow): void {
    this.state = row.state;
    this.url = row.url;
    this.fileName = row.fileName;
    this.path = row.path;
    this.total = row.total;
    this.downloaded = row.downloaded;
    this.bps = row.bps;
    this.connections = row.connections;
    this.resumable = row.resumable;
    this.speedLimit = row.speedLimit;
    this.error = row.error;
    this.createdAt = row.createdAt;
    this.completedAt = row.completedAt;
  }

  progress(item: ProgressItem): void {
    this.downloaded = item.downloaded;
    if (item.total !== null) this.total = item.total;
    this.bps = item.bps;
    this.connections = item.connections;
    if (item.segments !== null) this.segments = item.segments;
  }
}

export class DownloadStore {
  status = $state<EngineStatus>({ state: "starting" });
  settings = $state<AppSettings>({ speedLimit: null });
  /** Download ids, oldest first. Replaced only when the set changes. */
  ids = $state.raw<DownloadId[]>([]);
  /** The download whose file map streams (the details pane's). */
  focused = $state<DownloadId | null>(null);
  /** The last action that failed, for a quiet in-app notice. */
  lastError = $state<UiError | null>(null);

  readonly #api: EngineApi;
  // Not reactive on purpose: membership is tracked by `ids`, values by each
  // `Download`'s own fields, so a tick never invalidates the whole list.
  // eslint-disable-next-line svelte/prefer-svelte-reactivity
  readonly #items = new Map<DownloadId, Download>();
  // eslint-disable-next-line svelte/prefer-svelte-reactivity -- callbacks, not state
  readonly #listeners = new Set<(event: DownloadEvent) => void>();
  #connected = false;

  constructor(api: EngineApi) {
    this.#api = api;
  }

  /** Every download, oldest first. */
  get downloads(): Download[] {
    return this.ids.flatMap((id) => this.#items.get(id) ?? []);
  }

  get(id: DownloadId): Download | undefined {
    return this.#items.get(id);
  }

  /** Subscribes to the backend once; later calls do nothing. */
  async connect(): Promise<void> {
    if (this.#connected) return;
    this.#connected = true;
    await this.#api.subscribe((message) => this.apply(message));
  }

  /** Calls `listener` for what happens to downloads (for toasts, motion). */
  onEvent(listener: (event: DownloadEvent) => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  /** Applies one message from the backend. */
  apply(message: EngineMessage): void {
    switch (message.type) {
      case "snapshot":
        this.status = message.status;
        this.settings = message.settings;
        this.#replaceAll(message.downloads);
        break;
      case "status":
        this.status = message.status;
        break;
      case "settings":
        this.settings = message.settings;
        break;
      case "batch":
        this.#applyBatch(message);
        break;
    }
  }

  #replaceAll(rows: DownloadRow[]): void {
    // eslint-disable-next-line svelte/prefer-svelte-reactivity -- a local lookup
    const keep = new Set(rows.map((r) => r.id));
    for (const id of this.#items.keys()) {
      if (!keep.has(id)) this.#items.delete(id);
    }
    for (const row of rows) this.#upsert(row);
    this.#syncIds();
  }

  #applyBatch(batch: Extract<EngineMessage, { type: "batch" }>): void {
    let membership = false;
    for (const row of batch.rows) membership = this.#upsert(row) || membership;
    for (const id of batch.removed) membership = this.#items.delete(id) || membership;
    for (const item of batch.progress) this.#items.get(item.id)?.progress(item);
    if (membership) this.#syncIds();
    for (const event of batch.events) {
      for (const listener of this.#listeners) listener(event);
    }
  }

  /** Adds or updates a row; true if it was new. */
  #upsert(row: DownloadRow): boolean {
    const existing = this.#items.get(row.id);
    if (existing) {
      existing.update(row);
      return false;
    }
    this.#items.set(row.id, new Download(row));
    return true;
  }

  #syncIds(): void {
    this.ids = [...this.#items.keys()].sort((a, b) => a - b);
  }

  #failed<T>(result: Result<T>): result is { status: "error"; error: UiError } {
    if (result.status === "error") {
      this.lastError = result.error;
      return true;
    }
    return false;
  }

  // Actions -------------------------------------------------------------

  /** What a link points at, for the add sheet. */
  probe(url: string): Promise<Result<ProbePreview>> {
    return this.#api.commands.probeUrl(url);
  }

  /** Starts a download; its row appears at once. */
  async add(request: AddDownload): Promise<Result<DownloadRow>> {
    const result = await this.#api.commands.addDownload(request);
    if (!this.#failed(result) && this.#upsert(result.data)) this.#syncIds();
    return result;
  }

  async pause(id: DownloadId): Promise<Result<DownloadState>> {
    const d = this.#items.get(id);
    const before = d && { state: d.state, bps: d.bps, connections: d.connections };
    if (d) {
      d.state = "paused";
      d.bps = 0;
      d.connections = 0;
    }
    const result = await this.#api.commands.pauseDownload(id);
    if (this.#failed(result)) {
      if (d && before) Object.assign(d, before);
    } else if (d) {
      d.state = result.data;
    }
    return result;
  }

  /** Continues a stopped download; `restart` starts it from zero. */
  async resume(id: DownloadId, restart = false): Promise<Result<null>> {
    const d = this.#items.get(id);
    const before = d && { state: d.state, error: d.error };
    if (d) {
      d.state = "queued";
      d.error = null;
    }
    const result = await this.#api.commands.resumeDownload(id, restart);
    if (this.#failed(result) && d && before) Object.assign(d, before);
    return result;
  }

  /** Removes a download from the list, and its file with `deleteFile`. */
  async remove(id: DownloadId, deleteFile = false): Promise<Result<null>> {
    const d = this.#items.get(id);
    if (d) {
      this.#items.delete(id);
      this.#syncIds();
    }
    if (this.focused === id) this.focused = null;
    const result = await this.#api.commands.removeDownload(id, deleteFile);
    if (this.#failed(result) && d && result.error.key !== "error.unknown_download") {
      this.#items.set(id, d);
      this.#syncIds();
    }
    return result;
  }

  /** Gives a download whose link expired a new one; it stays paused. */
  async refreshLink(id: DownloadId, url: string): Promise<Result<null>> {
    const result = await this.#api.commands.refreshLink(id, url);
    this.#failed(result);
    return result;
  }

  /** Changes a speed limit in bytes per second; `null` removes it. */
  async setSpeedLimit(scope: LimitScope, bytesPerSecond: number | null): Promise<Result<null>> {
    const limit = bytesPerSecond && bytesPerSecond > 0 ? bytesPerSecond : null;
    const d = scope.kind === "download" ? this.#items.get(scope.id) : undefined;
    const before = scope.kind === "download" ? d?.speedLimit : this.settings.speedLimit;
    if (d) d.speedLimit = limit;
    else if (scope.kind === "global") this.settings = { ...this.settings, speedLimit: limit };
    const result = await this.#api.commands.setSpeedLimit(scope, bytesPerSecond);
    if (this.#failed(result)) {
      if (d) d.speedLimit = before ?? null;
      else if (scope.kind === "global")
        this.settings = { ...this.settings, speedLimit: before ?? null };
    }
    return result;
  }

  async updateSettings(settings: AppSettings): Promise<Result<AppSettings>> {
    const result = await this.#api.commands.updateSettings(settings);
    if (!this.#failed(result)) this.settings = result.data;
    return result;
  }

  /** The checksum, links and partial file of a download, for the details pane. */
  async details(id: DownloadId): Promise<Result<DownloadDetails>> {
    return this.#api.commands.downloadDetails(id);
  }

  /** Opens a completed download with its program. */
  async open(id: DownloadId): Promise<Result<null>> {
    const result = await this.#api.commands.openDownload(id);
    this.#failed(result);
    return result;
  }

  /** Shows a download's file (or its folder) in Explorer. */
  async showInFolder(id: DownloadId): Promise<Result<null>> {
    const result = await this.#api.commands.showDownloadInFolder(id);
    this.#failed(result);
    return result;
  }

  /** Streams the file map of `id` (or of nothing). */
  async focus(id: DownloadId | null): Promise<void> {
    if (this.focused === id) return;
    if (this.focused !== null) {
      const previous = this.#items.get(this.focused);
      if (previous) previous.segments = null;
    }
    this.focused = id;
    await this.#api.commands.setFocusedDownload(id);
  }

  /** Tells the backend whether the page is visible (no ticks while hidden). */
  async setVisible(visible: boolean): Promise<void> {
    await this.#api.commands.setUiVisible(visible);
  }
}
