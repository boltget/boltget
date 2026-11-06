// Development only: a simulated engine for the UI alone in a browser
// (`pnpm --filter @boltget/desktop dev`), for design review, screenshots and
// the 500-row smoothness check. Never part of a release build: index.ts
// imports it only when `import.meta.env.DEV` and outside Tauri.
//
// URL parameters: `?mock=500` seeds that many downloads (default 14, 0 for
// the empty state); `?active=20` how many of them run; `?engine=starting|
// locked|failed` holds the engine in that state; `?theme=light|dark`
// (read by native.ts) forces a theme; `?countdown=shut_down|sleep` shows a
// finished queue's countdown. Queues are kept, not run: nothing starts on
// its own.
import type {
  AddDownload,
  AppSettings,
  DownloadDetails,
  DownloadId,
  DownloadRow,
  DownloadState,
  EngineMessage,
  EngineStatus,
  FinishAction,
  FinishCountdown,
  LimitScope,
  ProgressItem,
  QueueId,
  QueueInfo,
  QueuePatch,
  SegmentSpan,
  SettingsPatch,
  ShiftDirection,
} from "../bindings";
import { placeOrder, queueOrder } from "../list/queue";
import { DEFAULT_SETTINGS } from "../settings/model";
import type { EngineApi, Result } from "./store.svelte";

const MIB = 1024 * 1024;
const TICK = 250;
const DOWNLOADS = String.raw`C:\Users\you\Downloads`;
const SEP = "\\";

const FILES: [string, string, number][] = [
  ["ubuntu-24.04.1-desktop-amd64.iso", "releases.ubuntu.com", 6114 * MIB],
  ["Blender-4.2.3-windows-x64.msi", "download.blender.org", 342 * MIB],
  ["big_buck_bunny_1080p_h264.mov", "download.blender.org", 691 * MIB],
  ["node-v22.11.0-x64.msi", "nodejs.org", 29 * MIB],
  ["OBS-Studio-31.0.0-Windows-Installer.exe", "github.com", 153 * MIB],
  ["lecture-07-distributed-systems.pdf", "cs.example.edu", 18 * MIB],
  ["kexp-live-session-2026.flac", "archive.org", 412 * MIB],
  ["VSCodeUserSetup-x64-1.95.3.exe", "update.code.visualstudio.com", 101 * MIB],
  ["Fedora-Workstation-Live-x86_64-41.iso", "download.fedoraproject.org", 2294 * MIB],
  ["LibreOffice_24.8.3_Win_x86-64.msi", "download.documentfoundation.org", 350 * MIB],
  ["sintel-4k-uhd.mkv", "media.xiph.org", 4410 * MIB],
  ["photos-backup-october.zip", "files.example.net", 1830 * MIB],
  ["annual-report-2025.pdf", "investors.example.com", 7 * MIB],
  ["podcast-episode-142.mp3", "cdn.example.fm", 84 * MIB],
  ["rust-1.82.0-x86_64-pc-windows-msvc.msi", "static.rust-lang.org", 318 * MIB],
  ["dataset-2026-10.tar.zst", "data.example.org", 12800 * MIB],
  ["gimp-2.10.38-setup.exe", "download.gimp.org", 312 * MIB],
  ["openstreetmap-europe-latest.osm.pbf", "download.geofabrik.de", 29_600 * MIB],
];

interface Sim {
  row: DownloadRow;
  segments: SegmentSpan[];
  /** Target speed while running, before limits. */
  rate: number;
}

function mulberry(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const ok = <T>(data: T): Promise<Result<T>> => Promise.resolve({ status: "ok", data });
const err = (key: string): Promise<Result<never>> =>
  Promise.resolve({ status: "error", error: { key, message: null } });

function initialSegments(total: number, connections: number, done: number): SegmentSpan[] {
  const size = Math.ceil(total / connections);
  const out: SegmentSpan[] = [];
  for (let i = 0; i < connections; i += 1) {
    const start = i * size;
    const end = Math.min(total, start + size);
    const received = Math.min(end - start, Math.floor((end - start) * done));
    out.push({ start, end, received, connection: received < end - start ? i : null });
  }
  return out;
}

export function mockEngineApi(search: string): EngineApi {
  const params = new URLSearchParams(search);
  const count = Number(params.get("mock") ?? 14);
  const activeCount = Number(params.get("active") ?? Math.min(4, Math.ceil(count / 4)));
  const forced = params.get("engine");
  const random = mulberry(7);
  const now = Date.now();
  const sims = new Map<DownloadId, Sim>();
  let settings: AppSettings = { ...DEFAULT_SETTINGS };
  let recent: string[] = [];
  let focused: DownloadId | null = null;
  let emit: (m: EngineMessage) => void = () => {};
  let nextId = 1;
  let queues: QueueInfo[] = [
    {
      id: 1,
      name: null,
      isDefault: true,
      maxConcurrent: 3,
      started: true,
      startTime: null,
      stopTime: null,
      onFinish: "nothing",
    },
  ];
  let nextQueue = 2;
  const forcedCountdown = params.get("countdown") as FinishAction | null;
  let countdown: FinishCountdown | null = forcedCountdown
    ? { queue: 1, action: forcedCountdown, deadline: Date.now() + 60_000 }
    : null;

  function make(i: number, state: DownloadState): Sim {
    const [name, host, size] = FILES[i % FILES.length] as [string, string, number];
    const round = Math.floor(i / FILES.length);
    const fileName = round ? name.replace(/(\.[^.]+)$/, ` (${round})$1`) : name;
    const id = nextId++;
    const age = state === "completed" ? (i * 3.7 + random() * 20) * 3_600_000 : i * 60_000;
    const done =
      state === "completed" ? 1 : state === "queued" ? 0 : 0.05 + random() * (i % 3 ? 0.5 : 0.85);
    const connections = state === "downloading" ? 4 + (i % 5) : 0;
    const segs = initialSegments(size, Math.max(4, connections), done);
    if (state !== "downloading") for (const s of segs) s.connection = null;
    const downloaded = segs.reduce((a, s) => a + s.received, 0);
    const error =
      state === "failed"
        ? { kind: "permanent" as const, key: "error.http.not_found", message: "HTTP 404 Not Found" }
        : state === "needs_new_link"
          ? {
              kind: "link_expired" as const,
              key: "error.link_expired",
              message: "HTTP 403 Forbidden (signed link expired)",
            }
          : state === "corrupt"
            ? {
                kind: "integrity" as const,
                key: "error.checksum_mismatch",
                message: "sha256 mismatch: expected 9f2c…, got 41ab…",
              }
            : null;
    return {
      row: {
        id,
        state,
        url: `https://${host}/${fileName.replaceAll(" ", "%20")}`,
        fileName,
        path: `C:\\Users\\you\\Downloads\\${fileName}`,
        total: size,
        downloaded,
        bps: 0,
        connections,
        resumable: i % 11 !== 5,
        speedLimit: null,
        error,
        createdAt: now - age - 600_000,
        completedAt: state === "completed" ? now - age : null,
        queue: 1,
        queuePosition: id,
        startAt: null,
      },
      segments: segs,
      rate: (6 + random() * 40) * MIB,
    };
  }

  // A realistic spread: a few running, some paused or stuck, most history.
  const pattern: DownloadState[] = ["paused", "failed", "needs_new_link", "queued", "corrupt"];
  for (let i = 0; i < count; i += 1) {
    let state: DownloadState;
    if (i < activeCount) state = "downloading";
    else if (i < activeCount + Math.min(5, Math.ceil(count / 6))) {
      state = pattern[(i - activeCount) % pattern.length] as DownloadState;
    } else state = "completed";
    const sim = make(i, state);
    sims.set(sim.row.id, sim);
  }

  const rows = () => [...sims.values()].map((s) => ({ ...s.row }));
  const status = (): EngineStatus =>
    forced === "starting"
      ? { state: "starting" }
      : forced === "locked"
        ? { state: "locked", dir: "C:\\Users\\you\\AppData\\Local\\Boltget" }
        : forced === "failed"
          ? { state: "failed", error: { key: "error.database", message: "disk I/O error" } }
          : { state: "ready" };

  function step(sim: Sim, dt: number): void {
    const limit = Math.min(sim.row.speedLimit ?? Infinity, settings.speedLimit ?? Infinity);
    const wobble = 0.8 + 0.4 * Math.sin(performance.now() / 1300 + sim.row.id);
    const bps = Math.min(limit, sim.rate * wobble);
    const working = sim.segments.filter((s) => s.connection !== null);
    if (working.length === 0) return;
    const share = (bps * dt) / working.length;
    for (const s of working) {
      const size = (s.end ?? 0) - s.start;
      s.received = Math.min(size, s.received + share * (0.7 + random() * 0.6));
      if (s.received >= size) {
        // A finished connection takes half of the largest part left.
        const freed = s.connection;
        s.connection = null;
        let largest: SegmentSpan | null = null;
        for (const o of sim.segments) {
          const left = (o.end ?? 0) - o.start - o.received;
          if (o.connection !== null && left > 4 * MIB) {
            if (!largest || left > (largest.end ?? 0) - largest.start - largest.received)
              largest = o;
          }
        }
        if (largest && freed !== null) {
          const from = largest.start + Math.floor(largest.received);
          const mid = from + Math.floor(((largest.end ?? 0) - from) / 2);
          sim.segments.push({ start: mid, end: largest.end, received: 0, connection: freed });
          largest.end = mid;
          sim.segments.sort((a, b) => a.start - b.start);
        }
      }
    }
    for (const s of sim.segments) s.received = Math.floor(s.received);
    sim.row.downloaded = sim.segments.reduce((a, s) => a + s.received, 0);
    sim.row.bps = Math.round(bps);
    sim.row.connections = sim.segments.filter((s) => s.connection !== null).length;
  }

  function tick(): void {
    if (status().state !== "ready") return;
    const progress: ProgressItem[] = [];
    const changed: DownloadRow[] = [];
    const events: Extract<EngineMessage, { type: "batch" }>["events"] = [];
    for (const sim of sims.values()) {
      if (sim.row.state !== "downloading") continue;
      step(sim, TICK / 1000);
      if (sim.row.downloaded >= (sim.row.total ?? Infinity)) {
        sim.row.state = "completed";
        sim.row.bps = 0;
        sim.row.connections = 0;
        sim.row.completedAt = Date.now();
        changed.push({ ...sim.row });
        events.push({ kind: "completed", id: sim.row.id, path: sim.row.path ?? "" });
        continue;
      }
      progress.push({
        id: sim.row.id,
        downloaded: sim.row.downloaded,
        total: sim.row.total,
        bps: sim.row.bps,
        connections: sim.row.connections,
        segments: sim.row.id === focused ? sim.segments.map((s) => ({ ...s })) : null,
      });
    }
    if (progress.length || changed.length) {
      emit({ type: "batch", rows: changed, removed: [], progress, events });
    }
  }

  function changeRow(sim: Sim, patch: Partial<DownloadRow>): void {
    Object.assign(sim.row, patch);
    emit({ type: "batch", rows: [{ ...sim.row }], removed: [], progress: [], events: [] });
  }

  function start(sim: Sim, restart: boolean): void {
    if (restart) sim.segments = initialSegments(sim.row.total ?? 64 * MIB, 4, 0);
    let n = 0;
    for (const s of sim.segments) {
      if (s.received < (s.end ?? 0) - s.start) s.connection = n++;
    }
    changeRow(sim, { state: "downloading", error: null, downloaded: sim.row.downloaded });
  }

  return {
    async subscribe(onMessage) {
      emit = onMessage;
      onMessage({
        type: "snapshot",
        status: status(),
        downloads: rows(),
        settings,
        queues,
        countdown,
      });
      setInterval(tick, TICK);
    },
    commands: {
      async addDownload(request: AddDownload) {
        let url: URL;
        try {
          url = new URL(request.url);
        } catch {
          return err("error.invalid_url");
        }
        const paused = request.paused ?? false;
        const sim = make(sims.size, paused ? "paused" : "downloading");
        const name =
          request.fileName ?? decodeURIComponent(url.pathname.split("/").pop() || url.hostname);
        const folder = request.directory ?? DOWNLOADS;
        sim.row.url = url.href;
        sim.row.fileName = name;
        sim.row.path = `${folder}${SEP}${name}`;
        sim.row.createdAt = Date.now();
        sim.row.completedAt = null;
        sim.row.downloaded = 0;
        sim.row.speedLimit = request.speedLimit ?? null;
        sim.row.queue = request.queue ?? 1;
        sim.row.queuePosition = sim.row.id;
        sim.row.startAt = request.startAt ?? null;
        sim.segments = initialSegments(sim.row.total ?? MIB, 4, 0);
        if (!paused) {
          const q = queues.find((x) => x.id === sim.row.queue);
          const busy = [...sims.values()].filter(
            (s) => s.row.queue === sim.row.queue && s.row.state === "downloading",
          ).length;
          if (q?.started && !sim.row.startAt && busy < q.maxConcurrent) start(sim, false);
          else sim.row.state = "queued";
        }
        sims.set(sim.row.id, sim);
        recent = [folder, ...recent.filter((f) => f !== folder)].slice(0, 3);
        return ok({ ...sim.row });
      },
      async addDefaults() {
        return ok({ downloadsFolder: DOWNLOADS, recentFolders: recent });
      },
      async checkDestination(directory: string | null, fileName: string) {
        // Names already in the list count as taken, like files on disk.
        const folder = directory ?? DOWNLOADS;
        const taken = new Set([...sims.values()].map((s) => s.row.path));
        let saved = fileName;
        const dot = fileName.lastIndexOf(".");
        const [stem, ext] =
          dot > 0 ? [fileName.slice(0, dot), fileName.slice(dot)] : [fileName, ""];
        for (let k = 1; taken.has(`${folder}${SEP}${saved}`); k += 1)
          saved = `${stem} (${k})${ext}`;
        return ok({
          fileName: saved,
          renamed: saved !== fileName,
          folderExists: true,
          freeBytes: 48 * 1024 * MIB,
        });
      },
      async chooseFolder() {
        return ok(String.raw`D:\Media\Incoming`);
      },
      async probeUrl(url: string) {
        // Simulated answers by link: "404", "html" (or a bare host), "auth",
        // "offline" and "slow" pick a case; anything else is a file.
        await new Promise((r) => setTimeout(r, /slow/.test(url) ? 2500 : 450));
        let u: URL;
        try {
          u = new URL(url);
        } catch {
          return err("error.invalid_url");
        }
        if (/404/.test(url)) return err("error.http.not_found");
        if (/auth/.test(url)) return err("error.http.unauthorized");
        if (/offline/.test(url)) return err("error.network");
        const page = /html/.test(url) || u.pathname === "/";
        const last = decodeURIComponent(u.pathname.split("/").pop() || "");
        return ok({
          finalUrl: url,
          fileName: page ? `${last || u.hostname}.html` : last || "download",
          size: page
            ? 1256
            : (([...url].reduce((a, c) => a + c.charCodeAt(0), 0) % 900) + 20) * MIB,
          resumable: !/once/.test(url),
          contentType: page ? "text/html" : "application/octet-stream",
          webPage: page,
          redirects: 0,
          checksum: /sum/.test(url)
            ? { algorithm: "sha256", digest: "9f86d08".padEnd(64, "0"), header: "repr-digest" }
            : null,
        });
      },
      async pauseDownload(id: DownloadId) {
        const sim = sims.get(id);
        if (!sim) return err("error.unknown_download");
        for (const s of sim.segments) s.connection = null;
        changeRow(sim, { state: "paused", bps: 0, connections: 0 });
        return ok("paused" as const);
      },
      async resumeDownload(id: DownloadId, restart: boolean) {
        const sim = sims.get(id);
        if (!sim) return err("error.unknown_download");
        if (sim.row.state === "completed") return err("error.invalid_state");
        setTimeout(() => start(sim, restart || sim.row.state === "corrupt"), 300);
        return ok(null);
      },
      async requeueDownload(id: DownloadId) {
        const sim = sims.get(id);
        if (!sim) return err("error.unknown_download");
        changeRow(sim, { state: "queued", error: null });
        return ok("queued" as const);
      },
      async setStartAt(id: DownloadId, at: number | null) {
        const sim = sims.get(id);
        if (!sim) return err("error.unknown_download");
        const state = at !== null && sim.row.state === "paused" ? "queued" : sim.row.state;
        changeRow(sim, { startAt: at, state });
        return ok(state);
      },
      async createQueue(name: string) {
        if (queues.some((q) => (q.name ?? "Main").toLowerCase() === name.trim().toLowerCase()))
          return err("error.queue.name_taken");
        const q: QueueInfo = {
          id: nextQueue++,
          name: name.trim(),
          isDefault: false,
          maxConcurrent: 3,
          started: true,
          startTime: null,
          stopTime: null,
          onFinish: "nothing",
        };
        queues = [...queues, q];
        emit({ type: "queues", queues });
        return ok(q);
      },
      async updateQueue(id: QueueId, patch: QueuePatch) {
        const q = queues.find((x) => x.id === id);
        if (!q) return err("error.unknown_queue");
        const next: QueueInfo = {
          ...q,
          name: patch.name?.trim() ?? q.name,
          maxConcurrent: patch.maxConcurrent ?? q.maxConcurrent,
          startTime: patch.times ? patch.times.start : q.startTime,
          stopTime: patch.times ? patch.times.stop : q.stopTime,
          onFinish: patch.onFinish ?? q.onFinish,
        };
        queues = queues.map((x) => (x.id === id ? next : x));
        emit({ type: "queues", queues });
        return ok(next);
      },
      async deleteQueue(id: QueueId) {
        queues = queues.filter((q) => q.id !== id || q.isDefault);
        for (const s of sims.values()) if (s.row.queue === id) changeRow(s, { queue: 1 });
        emit({ type: "queues", queues });
        return ok(null);
      },
      async startQueue(id: QueueId) {
        queues = queues.map((q) => (q.id === id ? { ...q, started: true } : q));
        emit({ type: "queues", queues });
        return ok(null);
      },
      async stopQueue(id: QueueId) {
        queues = queues.map((q) => (q.id === id ? { ...q, started: false } : q));
        emit({ type: "queues", queues });
        return ok(null);
      },
      async moveDownloads(ids: DownloadId[], queue: QueueId, before: DownloadId | null) {
        const items = [...sims.values()].map((s) => s.row);
        const order = placeOrder(queueOrder(items, queue), ids, before);
        const changed: DownloadRow[] = [];
        order.forEach((id, i) => {
          const sim = sims.get(id);
          if (sim) {
            sim.row.queue = queue;
            sim.row.queuePosition = i + 1;
            changed.push({ ...sim.row });
          }
        });
        emit({ type: "batch", rows: changed, removed: [], progress: [], events: [] });
        return ok(null);
      },
      async shiftDownload(id: DownloadId, direction: ShiftDirection) {
        const sim = sims.get(id);
        if (!sim) return err("error.unknown_download");
        const items = [...sims.values()].map((s) => s.row);
        const order = queueOrder(items, sim.row.queue);
        const i = order.indexOf(id);
        const before =
          direction === "top"
            ? (order[0] ?? null)
            : direction === "bottom"
              ? null
              : direction === "up"
                ? (order[i - 1] ?? order[0] ?? null)
                : (order[i + 2] ?? null);
        return this.moveDownloads([id], sim.row.queue, before);
      },
      async cancelFinishAction() {
        countdown = null;
        emit({ type: "countdown", countdown });
      },
      async removeDownload(id: DownloadId) {
        if (!sims.delete(id)) return err("error.unknown_download");
        emit({ type: "batch", rows: [], removed: [id], progress: [], events: [] });
        return ok(null);
      },
      async refreshLink(id: DownloadId, url: string) {
        const sim = sims.get(id);
        if (!sim) return err("error.unknown_download");
        changeRow(sim, { state: "paused", error: null, url });
        return ok(null);
      },
      async setSpeedLimit(scope: LimitScope, bytesPerSecond: number | null) {
        const limit = bytesPerSecond && bytesPerSecond > 0 ? bytesPerSecond : null;
        if (scope.kind === "global") {
          settings = { ...settings, speedLimit: limit };
          emit({ type: "settings", settings });
        } else {
          const sim = sims.get(scope.id);
          if (sim) changeRow(sim, { speedLimit: limit });
        }
        return ok(null);
      },
      async updateSettings(patch: SettingsPatch) {
        // The simulated backend checks what the page checks; the real one
        // checks again (src-tauri/src/settings.rs).
        await new Promise((r) => setTimeout(r, 60));
        const next: AppSettings = { ...settings };
        if (patch.maxConnections != null) next.maxConnections = patch.maxConnections;
        if (patch.downloadFolder != null) next.downloadFolder = patch.downloadFolder.trim() || null;
        if (patch.category) {
          const change = patch.category;
          next.categories = settings.categories.map((c) =>
            c.id !== change.id
              ? c
              : {
                  ...c,
                  extensions:
                    change.extensions == null || c.id === "other"
                      ? c.extensions
                      : change.extensions.split(/[\s,;]+/).filter(Boolean),
                  folder: change.folder == null ? c.folder : change.folder.trim() || null,
                },
          );
        }
        if (patch.proxy) next.proxy = patch.proxy;
        if (patch.theme != null) next.theme = patch.theme;
        if (patch.notifications != null) next.notifications = patch.notifications;
        if (patch.closeToTray != null) next.closeToTray = patch.closeToTray;
        if (patch.startWithWindows != null) next.startWithWindows = patch.startWithWindows;
        if (patch.clipboard != null) next.clipboard = patch.clipboard;
        settings = next;
        emit({ type: "settings", settings });
        return ok(settings);
      },
      async systemProxy() {
        return { proxy: params.get("proxy"), script: params.has("pac") };
      },
      async openDownloadsFolder() {
        return ok(null);
      },
      async quitApp() {},
      async setFocusedDownload(id: DownloadId | null) {
        focused = id;
      },
      async setUiVisible() {},
      async downloadDetails(id: DownloadId) {
        const sim = sims.get(id);
        if (!sim) return err("error.unknown_download");
        const details: DownloadDetails = {
          id,
          partialPath: sim.row.state === "completed" ? null : `${sim.row.path}.boltget`,
          checksum:
            id % 3 === 0
              ? {
                  algorithm: "sha256",
                  expected: "9f2c4e1b7a0d5c3e8f6a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f6a7b8",
                  actual:
                    sim.row.state === "completed"
                      ? "9f2c4e1b7a0d5c3e8f6a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f6a7b8"
                      : null,
                  header: id % 2 ? "Repr-Digest" : null,
                }
              : null,
          sources: [{ url: sim.row.url, role: "primary" }],
        };
        return ok(details);
      },
      async openDownload() {
        return ok(null);
      },
      async showDownloadInFolder() {
        return ok(null);
      },
    },
  };
}
