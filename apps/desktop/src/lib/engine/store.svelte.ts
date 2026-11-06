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
  AddDefaults,
  AddDownload,
  AppSettings,
  DestinationCheck,
  DownloadDetails,
  DownloadError,
  DownloadEvent,
  DownloadId,
  DownloadRow,
  DownloadState,
  EngineMessage,
  EngineStatus,
  FinishCountdown,
  LimitScope,
  ProbePreview,
  ProgressItem,
  QueueId,
  QueueInfo,
  QueuePatch,
  SegmentSpan,
  SettingsPatch,
  ShiftDirection,
  SystemProxyInfo,
  UiError,
  commands,
} from "../bindings";
import { setCategories } from "../list/categoryTable.svelte";
import { placeOrder } from "../list/queue";
import { DEFAULT_SETTINGS } from "../settings/model";

/** The result shape of every generated command that can fail. */
export type Result<T> = { status: "ok"; data: T } | { status: "error"; error: UiError };

/** What the store needs from the backend; tests pass a fake. */
export interface EngineApi {
  /** Starts the stream; `onMessage` first receives a snapshot. */
  subscribe(onMessage: (message: EngineMessage) => void): Promise<void>;
  commands: Pick<
    typeof commands,
    | "addDownload"
    | "addDefaults"
    | "checkDestination"
    | "chooseFolder"
    | "probeUrl"
    | "pauseDownload"
    | "resumeDownload"
    | "requeueDownload"
    | "setStartAt"
    | "createQueue"
    | "updateQueue"
    | "deleteQueue"
    | "startQueue"
    | "stopQueue"
    | "moveDownloads"
    | "shiftDownload"
    | "cancelFinishAction"
    | "removeDownload"
    | "refreshLink"
    | "setSpeedLimit"
    | "updateSettings"
    | "systemProxy"
    | "openDownloadsFolder"
    | "quitApp"
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
  /** The queue it belongs to, its place there, and when it starts on its own. */
  queue = $state<QueueId>(1);
  queuePosition = $state(0);
  startAt = $state<number | null>(null);
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
    this.queue = row.queue;
    this.queuePosition = row.queuePosition;
    this.startAt = row.startAt;
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
  settings = $state.raw<AppSettings>(DEFAULT_SETTINGS);
  /** The queues, the default first. */
  queues = $state.raw<QueueInfo[]>([]);
  /** A finished queue counting down to sleep or shut down. */
  countdown = $state.raw<FinishCountdown | null>(null);
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
        this.#setSettings(message.settings);
        this.queues = message.queues;
        this.countdown = message.countdown;
        this.#replaceAll(message.downloads);
        break;
      case "queues":
        this.queues = message.queues;
        break;
      case "countdown":
        this.countdown = message.countdown;
        break;
      case "status":
        this.status = message.status;
        break;
      case "settings":
        this.#setSettings(message.settings);
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

  /** The Downloads folder and the recent folders, for the add sheet. */
  addDefaults(): Promise<Result<AddDefaults>> {
    return this.#api.commands.addDefaults();
  }

  /** The name a download would be saved under, and the space free there. */
  checkDestination(directory: string | null, fileName: string): Promise<Result<DestinationCheck>> {
    return this.#api.commands.checkDestination(directory, fileName);
  }

  /** The system's folder picker; `null` when cancelled. */
  chooseFolder(start: string | null): Promise<Result<string | null>> {
    return this.#api.commands.chooseFolder(start);
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

  /** Lets stopped downloads wait in their queue again ("Resume all"). */
  async requeue(id: DownloadId): Promise<Result<DownloadState>> {
    const d = this.#items.get(id);
    const before = d && { state: d.state, error: d.error };
    if (d) {
      d.state = "queued";
      d.error = null;
    }
    const result = await this.#api.commands.requeueDownload(id);
    if (this.#failed(result)) {
      if (d && before) Object.assign(d, before);
    } else if (d) {
      d.state = result.data;
    }
    return result;
  }

  /** Starts a download at a time (Unix ms), or when its turn comes (`null`). */
  async setStartAt(id: DownloadId, at: number | null): Promise<Result<DownloadState>> {
    const d = this.#items.get(id);
    const before = d?.startAt ?? null;
    if (d) d.startAt = at;
    const result = await this.#api.commands.setStartAt(id, at);
    if (this.#failed(result)) {
      if (d) d.startAt = before;
    } else if (d) {
      d.state = result.data;
    }
    return result;
  }

  /** The queue with this id. */
  queue(id: QueueId): QueueInfo | undefined {
    return this.queues.find((q) => q.id === id);
  }

  /**
   * Creates a queue. A refusal (a name taken) is returned, not recorded:
   * the queue settings show it next to the field.
   */
  createQueue(name: string): Promise<Result<QueueInfo>> {
    return this.#api.commands.createQueue(name);
  }

  /** Changes a queue's settings; a refusal is returned, as for `createQueue`. */
  updateQueue(id: QueueId, patch: QueuePatch): Promise<Result<QueueInfo>> {
    return this.#api.commands.updateQueue(id, patch);
  }

  async deleteQueue(id: QueueId): Promise<Result<null>> {
    const result = await this.#api.commands.deleteQueue(id);
    this.#failed(result);
    return result;
  }

  /** Starts or stops a queue; the button follows at once. */
  async setQueueStarted(id: QueueId, started: boolean): Promise<Result<null>> {
    const flip = (on: boolean) =>
      (this.queues = this.queues.map((q) => (q.id === id ? { ...q, started: on } : q)));
    flip(started);
    const result = started
      ? await this.#api.commands.startQueue(id)
      : await this.#api.commands.stopQueue(id);
    if (this.#failed(result)) flip(!started);
    return result;
  }

  /**
   * Moves downloads into `queue` before `before` (or to the end), keeping
   * their order. The rows take their new places at once; the backend
   * confirms with the real positions.
   */
  async moveDownloads(
    ids: readonly DownloadId[],
    queue: QueueId,
    before: DownloadId | null,
    order: readonly DownloadId[],
  ): Promise<Result<null>> {
    const previous = ids.map((id) => {
      const d = this.#items.get(id);
      return d && { d, queue: d.queue, position: d.queuePosition };
    });
    const next = placeOrder(order, ids, before);
    next.forEach((id, i) => {
      const d = this.#items.get(id);
      if (d) {
        d.queue = queue;
        d.queuePosition = i + 1;
      }
    });
    const result = await this.#api.commands.moveDownloads([...ids], queue, before);
    if (this.#failed(result)) {
      for (const p of previous) {
        if (p) {
          p.d.queue = p.queue;
          p.d.queuePosition = p.position;
        }
      }
    }
    return result;
  }

  /** Moves a download within its queue (the menu's Move to top, say). */
  async shift(id: DownloadId, direction: ShiftDirection): Promise<Result<null>> {
    const result = await this.#api.commands.shiftDownload(id, direction);
    this.#failed(result);
    return result;
  }

  /** Cancels a finished queue's sleep or shut down. */
  async cancelFinish(): Promise<void> {
    this.countdown = null;
    await this.#api.commands.cancelFinishAction();
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
      else if (scope.kind === "global") {
        this.settings = { ...this.settings, speedLimit: before ?? null };
      }
    }
    return result;
  }

  #setSettings(settings: AppSettings): void {
    if (settings.categories !== this.settings.categories) setCategories(settings.categories);
    this.settings = settings;
  }

  /**
   * Changes some settings. A refusal is returned, not recorded as the last
   * error: Settings shows it next to the field.
   */
  async updateSettings(patch: SettingsPatch): Promise<Result<AppSettings>> {
    const result = await this.#api.commands.updateSettings(patch);
    if (result.status === "ok") this.#setSettings(result.data);
    return result;
  }

  /** What Windows' proxy settings say now. */
  systemProxy(): Promise<SystemProxyInfo> {
    return this.#api.commands.systemProxy();
  }

  /** Opens the default download folder in Explorer. */
  async openDownloadsFolder(): Promise<Result<null>> {
    const result = await this.#api.commands.openDownloadsFolder();
    this.#failed(result);
    return result;
  }

  /** Quits Boltget; downloads checkpoint and stay paused. */
  quit(): Promise<void> {
    return this.#api.commands.quitApp();
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
