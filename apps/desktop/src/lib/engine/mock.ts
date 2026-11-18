// Development only: a simulated engine for the UI alone in a browser
// (`pnpm --filter @boltget/desktop dev`), for design review, screenshots and
// the 500-row smoothness check. Never part of a release build: index.ts
// imports it only when `import.meta.env.DEV` and outside Tauri.
//
// URL parameters: `?mock=500` seeds that many downloads (default 14, 0 for
// the empty state); `?active=20` how many of them run; `?engine=starting|
// locked|failed` holds the engine in that state; `?theme=light|dark`
// (read by native.ts) forces a theme; `?countdown=shut_down|sleep` shows a
// finished queue's countdown; `?tools=installed` starts with the video
// tools installed (otherwise "Look for media" walks the install sheet
// first), `?install=fail` makes their install fail. Queues are kept, not
// run: nothing starts on its own. Media downloads run a simulated yt-dlp:
// they download, process for two seconds and complete; a playlist's entries
// run one at a time. Links with `list` are a 60-video playlist (two
// private, one deleted; `biglist`: 450, listed 200 at a time), and a
// `watch` link with `list` offers this video or the whole playlist. Clipboard
// monitoring is simulated by mock-clipboard.ts (`?clipboard=on` starts with
// it on). yt-dlp's failures are simulated by words in a page's link
// (`signin`, `unavailable`, `changed`, `network`, `processing`, `custom`,
// `broken`; with `fail`, the download fails that way instead of the look),
// `vp8` is a page whose streams cannot go into MP4 and `updated` one found
// after updating yt-dlp.
import type {
  AddDownload,
  AppSettings,
  BrowserIntegration,
  DownloadDetails,
  DownloadId,
  DownloadRow,
  DownloadState,
  EngineMessage,
  EngineStatus,
  FinishAction,
  FinishCountdown,
  LimitScope,
  MediaFormat,
  MediaEntry,
  MediaInstallEvent,
  MediaPlaylistMode,
  MediaPlaylistRequest,
  MediaPreset,
  MediaRequest,
  MediaSummary,
  MediaToolsInfo,
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
  /** A media download: when its processing ends (while processing). */
  media?: { until: number | null; name: string; playlist?: number };
}

const VIDEOS = String.raw`C:\Users\you\Videos`;

function preset(
  id: string,
  height: number | null,
  size: number,
  container: string,
  codecs: [string | null, string],
  approximate = false,
): MediaPreset {
  const audioOnly = height === null;
  return {
    preset: id,
    expression: audioOnly
      ? container === "m4a"
        ? "ba[ext=m4a]/ba/b"
        : "ba/b"
      : container === "mp4"
        ? `bv*[height<=${height}][ext=mp4]+ba[ext=m4a]/b[height<=${height}][ext=mp4]/bv*[height<=${height}]+ba/b[height<=${height}]`
        : `bv*[height<=${height}]+ba/b[height<=${height}]`,
    audioOnly,
    width: height === null ? null : Math.round((height * 16) / 9),
    height,
    fps: height === null ? null : height >= 1080 ? 60 : 30,
    container,
    videoCodec: codecs[0],
    audioCodec: codecs[1],
    size,
    sizeApproximate: approximate,
    keptOriginal: false,
  };
}

/**
 * yt-dlp's failures by link, as the backend sorts them (docs/VIDEO.md,
 * "Errors and pause/resume: as built"): a word of the link picks one.
 */
const SIMULATED_ERRORS: [RegExp, string, string][] = [
  [
    /signin/,
    "error.media.sign_in",
    "[youtube] dQw4w9WgXcQ: Sign in to confirm your age. This video may be inappropriate for some users. Use --cookies-from-browser or --cookies for the authentication.",
  ],
  [
    /unavailable/,
    "error.media.unavailable",
    "[youtube] dQw4w9WgXcQ: Video unavailable. This video has been removed by the uploader",
  ],
  [
    /changed/,
    "error.media.site_changed.current",
    "[vimeo] 123456789: Unable to extract info section; please report this issue on  https://github.com/yt-dlp/yt-dlp/issues?q= , filling out the appropriate issue template. Confirm you are on the latest version using  yt-dlp -U",
  ],
  [
    /network/,
    "error.media.network",
    "[generic] Unable to download webpage: <urlopen error [Errno 11001] getaddrinfo failed>",
  ],
  [/processing/, "error.media.processing", "Postprocessing: Conversion failed!"],
  [
    /custom/,
    "error.media.site_changed.custom",
    "[tiktok] 7300000000000000000: KeyError('itemInfo'); please report this issue on  https://github.com/yt-dlp/yt-dlp/issues?q= ",
  ],
  [/broken/, "error.media.no_media", "[generic] Unsupported URL: https://example.com/broken"],
];

function simulatedError(url: string): { key: string; message: string } | null {
  const hit = SIMULATED_ERRORS.find(([pattern]) => pattern.test(url));
  return hit ? { key: hit[1], message: hit[2] } : null;
}

function format(
  id: string,
  ext: string,
  height: number | null,
  vcodec: string | null,
  acodec: string | null,
  kbps: number,
  size: number | null,
  protocol = "https",
): MediaFormat {
  return {
    id,
    ext,
    width: height === null ? null : Math.round((height * 16) / 9),
    height,
    fps: height === null ? null : height >= 1080 ? 60 : 30,
    videoCodec: vcodec,
    audioCodec: acodec,
    bitrate: kbps,
    size,
    sizeApproximate: size !== null && size > 100 * MIB,
    protocol,
    note: null,
  };
}

/** A page with a video, as the simulated yt-dlp describes it. */
function videoPage(url: string, remembered: string | null): MediaSummary {
  return {
    url,
    site: (() => {
      try {
        return new URL(url).hostname.replace(/^www\./, "");
      } catch {
        return "";
      }
    })(),
    title: "A Video About Downloading",
    uploader: "Example Channel",
    duration: 212,
    thumbnail: "https://i.example.test/thumb.jpg",
    presets: [
      preset("best", 1080, 131 * MIB, "mp4", ["H.264", "AAC"], true),
      preset("720p", 720, 58 * MIB, "mp4", ["H.264", "AAC"]),
      preset("small", 480, 24 * MIB, "mp4", ["H.264", "AAC"]),
      preset("audio", null, 3.4 * MIB, "m4a", [null, "AAC"]),
    ],
    originalPresets: [
      preset("best", 2160, 402 * MIB, "webm", ["VP9", "Opus"], true),
      preset("1080p", 1080, 118 * MIB, "webm", ["VP9", "Opus"], true),
      preset("720p", 720, 51 * MIB, "webm", ["VP9", "Opus"]),
      preset("small", 480, 21 * MIB, "webm", ["VP9", "Opus"]),
      preset("audio", null, 3.1 * MIB, "opus", [null, "Opus"]),
    ],
    formats: [
      format("249", "webm", null, null, "Opus", 51, 1.3 * MIB),
      format("140", "m4a", null, null, "AAC", 129, 3.4 * MIB),
      format("251", "webm", null, null, "Opus", 135, 3.1 * MIB),
      format("18", "mp4", 360, "H.264", "AAC", 520, 13 * MIB),
      format("135", "mp4", 480, "H.264", null, 900, 21 * MIB),
      format("244", "webm", 480, "VP9", null, 780, 18 * MIB),
      format("136", "mp4", 720, "H.264", null, 2100, 55 * MIB),
      format("247", "webm", 720, "VP9", null, 1800, 48 * MIB),
      format("299", "mp4", 1080, "H.264", null, 4900, 128 * MIB),
      format("303", "webm", 1080, "VP9", null, 4400, 115 * MIB),
      format("96", "mp4", 1080, "H.264", "AAC", 5200, null, "m3u8_native"),
      format("315", "webm", 2160, "VP9", null, 15000, 399 * MIB),
    ],
    subtitles: [
      { code: "en", name: "English", automatic: false },
      { code: "de", name: "German", automatic: false },
      { code: "en", name: "English", automatic: true },
      { code: "tr", name: "Turkish", automatic: true },
    ],
    isPlaylist: false,
    entryCount: 0,
    playlist: null,
    inPlaylist: false,
    drm: false,
    remembered,
    updated: false,
  };
}

const ENTRY_TITLES = [
  "Choosing the wood",
  "Measuring twice",
  "Rough cuts",
  "Planing by hand",
  "Marking the joints",
  "Cutting tenons",
  "Chopping mortises",
  "Dry fit",
  "Shaping the back",
  "Glue-up day",
  "Sanding without tears",
  "The first coat",
];

/** The simulated playlist's entries (`count` in all): a few unavailable. */
function playlistEntries(count: number): MediaEntry[] {
  return Array.from({ length: count }, (_, i) => {
    const index = i + 1;
    const gone =
      index === 8 || index === 24 ? "[Private video]" : index === 41 ? "[Deleted video]" : null;
    return {
      index,
      id: `vid${String(index).padStart(5, "0")}`,
      title:
        gone ??
        `${ENTRY_TITLES[i % ENTRY_TITLES.length]}${i >= ENTRY_TITLES.length ? ` (part ${Math.floor(i / ENTRY_TITLES.length) + 1})` : ""}`,
      url: `https://www.youtube.com/watch?v=vid${String(index).padStart(5, "0")}`,
      duration: gone ? null : 180 + ((index * 137) % 1500),
      uploader: null,
      thumbnail: gone ? null : `https://i.example.test/vid${index}.jpg`,
      available: gone === null,
    };
  });
}

/** A thumbnail for the simulated page, as the backend would send it. */
const THUMBNAIL = `data:image/svg+xml;base64,${btoa(
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 160 90"><defs><linearGradient id="g" x2="1" y2="1"><stop offset="0" stop-color="#3b5bdb"/><stop offset="1" stop-color="#0b7285"/></linearGradient></defs><rect width="160" height="90" fill="url(#g)"/><path d="M68 30v30l26-15z" fill="#fff" fill-opacity=".85"/></svg>',
)}`;

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
  let settings: AppSettings = { ...DEFAULT_SETTINGS, clipboard: params.get("clipboard") === "on" };
  // `?browsers=missing` shows the browsers needing repair.
  let browsers: BrowserIntegration = {
    supported: true,
    browsers: (["chrome", "edge", "firefox"] as const).map((browser) => ({
      browser,
      state: params.get("browsers") === "missing" ? ("missing" as const) : ("registered" as const),
    })),
  };
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
  // The video tools and the media jobs (row ids from 2^50, as the backend's).
  let tools: MediaToolsInfo = {
    ready: params.get("tools") === "installed",
    channel: "stable",
    tools: [
      {
        tool: "ytDlp",
        version: null,
        previous: null,
        lastCheck: null,
        customPath: null,
        customVersion: null,
        supported: true,
      },
      {
        tool: "ffmpeg",
        version: null,
        previous: null,
        lastCheck: null,
        customPath: null,
        customVersion: null,
        supported: true,
      },
    ],
  };
  const installedVersions = { ytDlp: "2026.10.26", ffmpeg: "n9.0.2-39-g7adfbe40dc" };
  if (tools.ready) {
    tools = {
      ...tools,
      tools: tools.tools.map((x) => ({
        ...x,
        version: installedVersions[x.tool],
        lastCheck: Date.now() - 3_600_000,
      })),
    };
  }
  let nextMedia = 1;
  let nextPlaylist = 1;
  const rememberedPresets = new Map<string, string>();
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
        private: false,
        media: null,
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

  /** A media download's tick: download, process for a while, complete. */
  function stepMedia(
    sim: Sim,
    progress: ProgressItem[],
    changed: DownloadRow[],
    events: Extract<EngineMessage, { type: "batch" }>["events"],
  ): void {
    const media = sim.media;
    if (!media) return;
    if (sim.row.state === "processing") {
      if (media.until !== null && Date.now() >= media.until) {
        media.until = null;
        sim.row.state = "completed";
        sim.row.downloaded = sim.row.total ?? sim.row.downloaded;
        sim.row.completedAt = Date.now();
        sim.row.fileName = media.name;
        sim.row.path = `${VIDEOS}${SEP}${media.name}`;
        changed.push({ ...sim.row });
        events.push({ kind: "completed", id: sim.row.id, path: sim.row.path });
        if (media.playlist !== undefined) {
          const next = nextOfPlaylist(media.playlist);
          if (next) {
            next.row.state = "downloading";
            changed.push({ ...next.row });
          }
        }
      }
      return;
    }
    if (sim.row.state !== "downloading") return;
    const wobble = 0.8 + 0.4 * Math.sin(performance.now() / 900 + sim.row.id);
    sim.row.downloaded = Math.min(
      sim.row.total ?? Infinity,
      sim.row.downloaded + (sim.rate * wobble * TICK) / 1000,
    );
    sim.row.bps = Math.round(sim.rate * wobble);
    if (sim.row.downloaded >= (sim.row.total ?? Infinity)) {
      sim.row.state = "processing";
      sim.row.bps = 0;
      media.until = Date.now() + 2000;
      changed.push({ ...sim.row });
      events.push({ kind: "stateChanged", id: sim.row.id, state: "processing", error: null });
      return;
    }
    progress.push({
      id: sim.row.id,
      downloaded: Math.floor(sim.row.downloaded),
      total: sim.row.total,
      bps: sim.row.bps,
      connections: 0,
      segments: null,
    });
  }

  function tick(): void {
    if (status().state !== "ready") return;
    const progress: ProgressItem[] = [];
    const changed: DownloadRow[] = [];
    const events: Extract<EngineMessage, { type: "batch" }>["events"] = [];
    for (const sim of sims.values()) {
      if (sim.media) {
        stepMedia(sim, progress, changed, events);
        continue;
      }
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

  /**
   * The entry of a playlist to start next, while none of its entries runs
   * (a playlist downloads one video at a time).
   */
  function nextOfPlaylist(playlist: number): Sim | null {
    const entries = [...sims.values()].filter((x) => x.media?.playlist === playlist);
    if (entries.some((x) => x.row.state === "downloading" || x.row.state === "processing")) {
      return null;
    }
    return (
      entries
        .filter((x) => x.row.state === "queued")
        .sort(
          (a, b) => (a.row.media?.playlist?.index ?? 0) - (b.row.media?.playlist?.index ?? 0),
        )[0] ?? null
    );
  }

  function advancePlaylist(playlist: number | undefined): void {
    if (playlist === undefined) return;
    const next = nextOfPlaylist(playlist);
    if (next) changeRow(next, { state: "downloading", error: null });
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

  async function wait(ms: number): Promise<void> {
    await new Promise((r) => setTimeout(r, ms));
  }

  return {
    async installMedia(onEvent: (event: MediaInstallEvent) => void) {
      const sizes = { ytDlp: 37.9 * MIB, ffmpeg: 86.2 * MIB };
      for (const tool of ["ytDlp", "ffmpeg"] as const) {
        onEvent({ tool, step: "checking", done: 0, total: null });
        await wait(400);
        for (let i = 1; i <= 12; i += 1) {
          onEvent({
            tool,
            step: "downloading",
            done: Math.round((sizes[tool] * i) / 12),
            total: sizes[tool],
          });
          await wait(150);
          if (params.get("install") === "fail" && tool === "ffmpeg" && i === 6) {
            return err("error.media.download");
          }
        }
        for (const step of ["verifying", "unpacking", "testing"] as const) {
          onEvent({ tool, step, done: 0, total: null });
          await wait(350);
        }
        onEvent({ tool, step: "done", done: 0, total: null });
      }
      tools = {
        ...tools,
        ready: true,
        tools: tools.tools.map((x) => ({
          ...x,
          version: installedVersions[x.tool],
          lastCheck: Date.now(),
        })),
      };
      return ok(tools);
    },
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
      async probeMedia(
        url: string,
        _page: number | null,
        playlist: MediaPlaylistMode | null,
        start: number | null,
      ) {
        // Simulated answers by link: "tools" (not installed), "nomedia",
        // "drm", "list" (a playlist), "vp8" (a page whose streams cannot go
        // into MP4), "updated" (found after updating yt-dlp), and yt-dlp's
        // failures ("signin", "unavailable", "changed", "network",
        // "processing", "custom", "broken") pick a case; anything else is a
        // video. Until the tools are installed (Settings, or the install
        // sheet) every page answers notInstalled.
        await wait(/slow/.test(url) ? 2500 : 700);
        if (!tools.ready || /tools/.test(url)) return ok({ kind: "notInstalled" as const });
        const failure = simulatedError(url);
        if (failure && !/fail/.test(url)) {
          return Promise.resolve({ status: "error" as const, error: failure });
        }
        const page = videoPage(url, null);
        const empty: MediaSummary = {
          ...page,
          title: null,
          uploader: null,
          duration: null,
          thumbnail: null,
          presets: [],
          originalPresets: [],
          formats: [],
          subtitles: [],
        };
        if (/nomedia/.test(url)) return ok({ kind: "found" as const, media: empty });
        if (/drm/.test(url)) {
          return ok({
            kind: "found" as const,
            media: { ...empty, title: "Premium Series S1E1", duration: 2940, drm: true },
          });
        }
        const watchInList = /watch/.test(url) && /[?&]list=/.test(url);
        if (/list/.test(url) && (!watchInList || playlist === "entries")) {
          const all = playlistEntries(/biglist/.test(url) ? 450 : 60);
          const from = Math.max(1, start ?? 1);
          const entries = all.slice(from - 1, from - 1 + 200);
          return ok({
            kind: "found" as const,
            media: {
              ...empty,
              title: "Weekend projects: building a chair",
              uploader: "Example Channel",
              isPlaylist: true,
              entryCount: all.length,
              playlist: {
                id: "PLmock",
                start: from,
                entries,
                more: from - 1 + entries.length < all.length,
                folder: "Weekend projects_ building a chair",
              },
            },
          });
        }
        if (/vp8/.test(url)) {
          // Wikimedia Commons: one WebM with VP8 and Vorbis.
          const webm = {
            ...preset("best", 720, 1.6 * MIB, "webm", ["VP8", "Vorbis"]),
            expression: "bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/bv*+ba/b",
          };
          return ok({
            kind: "found" as const,
            media: {
              ...page,
              title: "Urmia lake drought",
              presets: [
                { ...webm, keptOriginal: true },
                preset("audio", null, 1.6 * MIB, "m4a", [null, "Vorbis"], true),
              ],
              originalPresets: [
                { ...webm, expression: "bv*+ba/b" },
                preset("audio", null, 1.6 * MIB, "ogg", [null, "Vorbis"], true),
              ],
              formats: [format("original", "webm", 720, "VP8", "Vorbis", 380, 1.6 * MIB)],
              subtitles: [],
            },
          });
        }
        return ok({
          kind: "found" as const,
          media: {
            ...page,
            inPlaylist: watchInList,
            remembered: rememberedPresets.get(page.site) ?? null,
            updated: /updated/.test(url),
          },
        });
      },
      async mediaThumbnail() {
        await wait(300);
        return ok(THUMBNAIL);
      },
      async addMediaDownload(request: MediaRequest) {
        const id = 2 ** 50 + nextMedia++;
        const title = request.title ?? "video";
        const audio = request.preset === "audio";
        const ext = audio
          ? request.container === "mp4"
            ? "m4a"
            : "opus"
          : request.container === "mp4"
            ? "mp4"
            : "webm";
        const total = request.size ?? 64 * MIB;
        const sim: Sim = {
          row: {
            id,
            state: "downloading",
            url: request.url.split("?")[0] ?? request.url,
            fileName: `${title}.${audio ? "m4a" : request.container === "mp4" ? "mp4" : "mkv"}`,
            path: null,
            total,
            downloaded: 0,
            bps: 0,
            connections: 0,
            resumable: true,
            speedLimit: null,
            error: null,
            createdAt: Date.now(),
            completedAt: null,
            queue: 0,
            queuePosition: Number.MAX_SAFE_INTEGER,
            startAt: null,
            private: false,
            media: {
              title: request.title ?? null,
              preset: request.preset ?? null,
              playlist: null,
              // How it went, as the details pane says it.
              note: /vp8/.test(request.url)
                ? "keptOriginal"
                : /updated/.test(request.url)
                  ? "updated"
                  : null,
            },
          },
          segments: [],
          rate: (4 + random() * 10) * MIB,
          media: { until: null, name: `${title} [dQw4w9WgXcQ].${ext}` },
        };
        if (/fail/.test(request.url)) {
          // "fail" with one of the failures' words fails that way; alone,
          // as "other".
          const failure = simulatedError(request.url) ?? {
            key: "error.media.other",
            message:
              "[youtube] dQw4w9WgXcQ: Requested format is not available. Use --list-formats for a list of available formats",
          };
          sim.row.state = "failed";
          sim.row.error = {
            kind: failure.key === "error.media.network" ? "transient" : "permanent",
            ...failure,
          };
        }
        sims.set(id, sim);
        if (request.site && request.preset) rememberedPresets.set(request.site, request.preset);
        return ok({ ...sim.row });
      },
      async addMediaPlaylist(request: MediaPlaylistRequest) {
        await wait(200);
        if (request.entries.length === 0) return err("error.media.entries");
        const playlist = nextPlaylist++;
        const audio = request.preset === "audio";
        const ext = audio ? "m4a" : request.container === "mp4" ? "mp4" : "webm";
        const width = Math.max(2, String(request.entryCount).length);
        const rows: DownloadRow[] = [];
        for (const [i, entry] of request.entries.entries()) {
          const id = 2 ** 50 + nextMedia++;
          const place = String(entry.index).padStart(width, "0");
          const title = entry.title ?? "video";
          const sim: Sim = {
            row: {
              id,
              state: i === 0 ? "downloading" : "queued",
              url: entry.url.split("?")[0] ?? entry.url,
              fileName: `${place} - ${title}.${ext}`,
              path: null,
              total: (20 + ((entry.index * 37) % 90)) * MIB,
              downloaded: 0,
              bps: 0,
              connections: 0,
              resumable: true,
              speedLimit: null,
              error: null,
              createdAt: Date.now(),
              completedAt: null,
              queue: 0,
              queuePosition: Number.MAX_SAFE_INTEGER,
              startAt: null,
              private: false,
              media: {
                title: entry.title ?? null,
                preset: request.preset,
                playlist: { id: playlist, title: request.title ?? null, index: entry.index },
                note: null,
              },
            },
            segments: [],
            rate: (6 + random() * 10) * MIB,
            media: {
              until: null,
              name: `${place} - ${title} [vid${entry.index}].${ext}`,
              playlist,
            },
          };
          sims.set(id, sim);
          rows.push({ ...sim.row });
        }
        return ok(rows);
      },
      async pauseMediaPlaylist(playlist: number) {
        for (const sim of sims.values()) {
          if (sim.media?.playlist !== playlist) continue;
          // One that is processing finishes (it is not paused).
          if (["queued", "downloading"].includes(sim.row.state)) {
            sim.media.until = null;
            changeRow(sim, { state: "paused", bps: 0 });
          }
        }
        return ok(null);
      },
      async resumeMediaPlaylist(playlist: number) {
        for (const sim of sims.values()) {
          if (sim.media?.playlist !== playlist) continue;
          if (sim.row.state === "paused" || sim.row.state === "failed") {
            changeRow(sim, { state: "queued", error: null });
          }
        }
        advancePlaylist(playlist);
        return ok(null);
      },
      async mediaTools() {
        await wait(150);
        return ok(tools);
      },
      async mediaInstallPlan() {
        await wait(500);
        return ok(
          tools.tools
            .filter((x) => !x.version && !x.customPath)
            .map((x) => ({
              tool: x.tool,
              version: installedVersions[x.tool],
              size: x.tool === "ytDlp" ? 37.9 * MIB : 86.2 * MIB,
            })),
        );
      },
      async updateMediaTools() {
        await wait(1200);
        tools = {
          ...tools,
          tools: tools.tools.map((x) => ({
            ...x,
            lastCheck: x.version ? Date.now() : x.lastCheck,
          })),
        };
        return ok(tools);
      },
      async removeMediaTools() {
        if (
          [...sims.values()].some(
            (x) => x.media && (x.row.state === "downloading" || x.row.state === "processing"),
          )
        ) {
          return err("error.media.in_use");
        }
        await wait(300);
        tools = {
          ...tools,
          tools: tools.tools.map((x) => ({ ...x, version: null, previous: null, lastCheck: null })),
        };
        tools.ready = tools.tools.every((x) => x.customPath);
        return ok(tools);
      },
      async setMediaChannel(channel) {
        tools = { ...tools, channel };
        return ok(tools);
      },
      async setMediaToolPath(tool, path) {
        await wait(400);
        if (path && !/^[a-z]:\\.+\.exe$/i.test(path.trim()))
          return err("settings.problem.program_path");
        if (path && /missing/i.test(path)) return err("settings.problem.program_missing");
        tools = {
          ...tools,
          tools: tools.tools.map((x) =>
            x.tool === tool
              ? {
                  ...x,
                  customPath: path?.trim() || null,
                  customVersion: path
                    ? tool === "ytDlp"
                      ? "2026.11.01"
                      : "ffmpeg version 7.1"
                    : null,
                }
              : x,
          ),
        };
        tools.ready = tools.tools.every((x) => x.version || x.customPath);
        const key = tool === "ytDlp" ? "ytDlpPath" : "ffmpegPath";
        settings = { ...settings, video: { ...settings.video, [key]: path?.trim() || null } };
        emit({ type: "settings", settings });
        return ok(tools);
      },
      async openMediaProject() {
        return ok(null);
      },
      async mediaHintLink() {
        return ok(null);
      },
      async pauseDownload(id: DownloadId) {
        const sim = sims.get(id);
        if (!sim) return err("error.unknown_download");
        if (sim.media) sim.media.until = null;
        for (const s of sim.segments) s.connection = null;
        changeRow(sim, { state: "paused", bps: 0, connections: 0 });
        advancePlaylist(sim.media?.playlist);
        return ok("paused" as const);
      },
      async resumeDownload(id: DownloadId, restart: boolean) {
        const sim = sims.get(id);
        if (!sim) return err("error.unknown_download");
        if (sim.row.state === "completed") return err("error.invalid_state");
        if (sim.media) {
          const playlist = sim.media.playlist;
          if (playlist !== undefined) {
            // An entry waits while another of its playlist runs.
            changeRow(sim, { state: "queued", error: null });
            setTimeout(() => advancePlaylist(playlist), 300);
            return ok(null);
          }
          setTimeout(() => changeRow(sim, { state: "downloading", error: null }), 300);
          return ok(null);
        }
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
        const playlist = sims.get(id)?.media?.playlist;
        if (!sims.delete(id)) return err("error.unknown_download");
        emit({ type: "batch", rows: [], removed: [id], progress: [], events: [] });
        advancePlaylist(playlist);
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
        if (patch.clipboardVideo != null) next.clipboardVideo = patch.clipboardVideo;
        if (patch.browserAsk != null) next.browserAsk = patch.browserAsk;
        if (patch.forgetPrivate != null) next.forgetPrivate = patch.forgetPrivate;
        if (patch.video?.container)
          next.video = { ...next.video, container: patch.video.container };
        if (patch.capture) {
          const c = patch.capture;
          next.capture = {
            enabled: c.enabled ?? settings.capture.enabled,
            minSize: c.minSize ?? settings.capture.minSize,
            extensions: c.extensions ?? settings.capture.extensions,
            neverSites: c.neverSites ?? settings.capture.neverSites,
            alwaysSites: c.alwaysSites ?? settings.capture.alwaysSites,
            privateWindows: c.privateWindows ?? settings.capture.privateWindows,
          };
        }
        settings = next;
        emit({ type: "settings", settings });
        return ok(settings);
      },
      async systemProxy() {
        return { proxy: params.get("proxy"), script: params.has("pac") };
      },
      async browserIntegration() {
        return browsers;
      },
      async repairBrowserIntegration() {
        browsers = {
          supported: true,
          browsers: browsers.browsers.map((b) => ({ ...b, state: "registered" as const })),
        };
        return ok(browsers);
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
