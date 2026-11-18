// The media picker and the first-use install sheet (docs/VIDEO.md, "Format
// and quality picker UX"; docs/APP-DESIGN.md, "Video as built"): what they
// show for a page's presets, formats, subtitles and the tools' install.
// Pure functions over the backend's values, tested without a DOM.
import { t, type PlainMessageKey } from "@boltget/ui/messages";
import type {
  MediaContainer,
  MediaFormat,
  MediaHintInfo,
  MediaHintKind,
  MediaInstallEvent,
  MediaInstallStep,
  MediaPreset,
  MediaSubtitle,
  MediaSummary,
  MediaTool,
  MediaToolSize,
} from "../bindings";
import { formatBytes } from "../format";

/** A video's length: `3:32`, `1:02:05`. */
export function mediaLength(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  const two = (n: number) => String(n).padStart(2, "0");
  const h = Math.floor(s / 3600);
  const m = Math.floor(s / 60) % 60;
  return h > 0 ? `${h}:${two(m)}:${two(s % 60)}` : `${m}:${two(s % 60)}`;
}

/** A size as the picker shows it: "about 31 MiB" when estimated, `null` when unknown. */
export function sizeText(size: number | null, approximate: boolean): string | null {
  if (size === null) return null;
  const text = formatBytes(size);
  return approximate ? t("media.about", { size: text }) : text;
}

const PRESET_NAMES: Record<string, PlainMessageKey> = {
  best: "media.preset.best",
  "1080p": "media.preset.1080p",
  "720p": "media.preset.720p",
  small: "media.preset.small",
  audio: "media.preset.audio",
};

/** A preset's name: "Best quality", "1080p", "Smaller file", ... */
export function presetName(preset: string): string {
  const key = PRESET_NAMES[preset];
  return key ? t(key) : preset;
}

/** `1080p`, `1080p60` (above 30 fps), or `null` without video. */
export function resolution(height: number | null, fps: number | null): string | null {
  if (height === null) return null;
  return fps !== null && fps > 30.5 ? `${height}p${Math.round(fps)}` : `${height}p`;
}

/** What a preset gives, in a few parts: `1080p · MP4 · H.264 + AAC · about 31 MiB`. */
export function presetDetails(p: MediaPreset): string[] {
  const parts: string[] = [];
  const res = resolution(p.audioOnly ? null : p.height, p.fps);
  if (res) parts.push(res);
  parts.push(p.container.toUpperCase());
  const codecs = [p.audioOnly ? null : p.videoCodec, p.audioCodec].filter((c): c is string => !!c);
  if (codecs.length) parts.push(codecs.join(" + "));
  const size = sizeText(p.size, p.sizeApproximate);
  if (size) parts.push(size);
  return parts;
}

/** The presets for the container toggle (collapsed ones are already gone). */
export function presetsFor(media: MediaSummary, container: MediaContainer): MediaPreset[] {
  return container === "mp4" ? media.presets : media.originalPresets;
}

/**
 * The preset chosen when the picker opens or the toggle flips: the one
 * chosen before if it is still there, else the one last used on this site,
 * else the first. A preset collapsed into another (1080p into Best quality)
 * is not there, so the first, which gives the same file, is taken.
 */
export function initialPreset(
  list: readonly MediaPreset[],
  current: string | null,
  remembered: string | null,
): string | null {
  for (const id of [current, remembered]) {
    if (id && list.some((p) => p.preset === id)) return id;
  }
  return list[0]?.preset ?? null;
}

/**
 * What a media failure adds to its message (docs/VIDEO.md, "Errors and
 * pause/resume: as built"): how to sign in so the cookies come along, or
 * where to update one's own yt-dlp. The message itself is the key's.
 */
export function failureHint(key: string): PlainMessageKey | null {
  switch (key) {
    case "error.media.sign_in":
      return "media.error.sign_in.hint";
    case "error.media.site_changed.custom":
      return "media.error.custom.hint";
    default:
      return null;
  }
}

/**
 * The container a single page's download asks for: a preset whose streams
 * cannot go into MP4 keeps its own ("Prefer original"), so FFmpeg is never
 * asked for what it cannot do; a typed expression keeps the switch's.
 */
export function requestContainer(
  container: MediaContainer,
  chosen: MediaPreset | null,
  typed: boolean,
): MediaContainer {
  return !typed && chosen?.keptOriginal ? "original" : container;
}

/** What the picker shows instead of presets, if anything. */
export type PickerNotice = "playlist" | "drm" | "none" | null;

export function pickerNotice(media: MediaSummary): PickerNotice {
  if (media.isPlaylist) return "playlist";
  if (media.drm) return "drm";
  if (media.presets.length === 0 && media.originalPresets.length === 0) return "none";
  return null;
}

/**
 * The page's hints the sheet offers (docs/VIDEO.md, "Hints"): for a page the
 * browser sent with some, when yt-dlp could not read `current` or found
 * nothing to download there (not for a playlist or DRM); every hint but
 * `current` itself, so a hint that failed leaves the others.
 */
export function offeredHints(
  hints: readonly MediaHintInfo[],
  outcome: "failed" | PickerNotice,
  current: string,
): MediaHintInfo[] {
  if (outcome !== "failed" && outcome !== "none") return [];
  return hints.filter((h) => h.url !== current);
}

const HINT_KINDS: Record<MediaHintKind, PlainMessageKey> = {
  hls: "media.hint.hls",
  dash: "media.hint.dash",
  file: "media.hint.file",
};

/** A resolution named in a link (`…/720p/…`, `…_1920x1080.m3u8`), if any. */
export function linkResolution(url: string): string | null {
  let path: string;
  try {
    path = decodeURIComponent(new URL(url).pathname);
  } catch {
    return null;
  }
  const p = /(?:^|[^0-9a-z])(\d{3,4})p(?:\d{2})?(?![0-9a-z])/i.exec(path);
  if (p?.[1]) return `${p[1]}p`;
  const wh = /(?:^|[^0-9])\d{3,4}x(\d{3,4})(?![0-9])/i.exec(path);
  return wh?.[1] ? `${wh[1]}p` : null;
}

/** One hint as the sheet lists it: what it is, then where and how big. */
export function hintLine(hint: MediaHintInfo): { name: string; host: string; details: string[] } {
  let name = t(HINT_KINDS[hint.kind]);
  if (hint.kind === "file" && hint.mime?.toLowerCase().startsWith("audio/")) {
    name = t("media.hint.audio");
  }
  const details: string[] = [];
  const res = linkResolution(hint.url);
  if (res) details.push(res);
  let host = "";
  try {
    host = new URL(hint.url).hostname;
  } catch {
    // Validated by the backend; nothing to show.
  }
  if (host) details.push(host);
  if (hint.size !== null) details.push(formatBytes(hint.size));
  return { name, host, details };
}

/** One subtitle choice of the dropdown. */
export interface SubtitleOption {
  value: string;
  label: string;
  /** The language code, and whether it is automatic captions. */
  code: string;
  automatic: boolean;
}

/**
 * The subtitle languages, the site's own first, then automatic captions
 * (marked), each sorted by name. Values are unique even when a language has
 * both kinds.
 */
export function subtitleOptions(subs: readonly MediaSubtitle[]): SubtitleOption[] {
  const byName = (a: MediaSubtitle, b: MediaSubtitle) =>
    (a.name ?? a.code).localeCompare(b.name ?? b.code);
  const own = subs.filter((s) => !s.automatic).sort(byName);
  const auto = subs.filter((s) => s.automatic).sort(byName);
  return [...own, ...auto].map((s) => {
    const name = s.name ?? s.code;
    return {
      value: `${s.automatic ? "auto" : "own"}:${s.code}`,
      label: s.automatic ? t("media.subtitles.automatic", { name }) : name,
      code: s.code,
      automatic: s.automatic,
    };
  });
}

/** A row of the advanced table, as text. */
export interface FormatRow {
  id: string;
  /** `1920×1080`, or "Audio only". */
  resolution: string;
  fps: string;
  /** `H.264 + AAC`, `Opus`. */
  codec: string;
  bitrate: string;
  size: string;
  protocol: string;
  /** It can be the video stream (it has video). */
  video: boolean;
  /** It can be the audio stream (audio and no video). */
  audio: boolean;
}

const PROTOCOLS: Record<string, string> = {
  https: "HTTPS",
  http: "HTTP",
  m3u8: "HLS",
  m3u8_native: "HLS",
  http_dash_segments: "DASH",
  dash_frag_urls: "DASH",
  f4m: "HDS",
  ism: "Smooth",
};

/** A protocol in everyday words: `HTTPS`, `HLS`, `DASH`. */
export function protocolName(protocol: string | null): string {
  if (!protocol) return "";
  // yt-dlp joins the protocols of merged formats with "+".
  return protocol
    .split("+")
    .map((p) => PROTOCOLS[p] ?? p.toUpperCase())
    .join(" + ");
}

export function formatRows(formats: readonly MediaFormat[]): FormatRow[] {
  return formats.map((f) => {
    const hasVideo = f.videoCodec !== null;
    const codecs = [f.videoCodec, f.audioCodec].filter((c): c is string => !!c);
    return {
      id: f.id,
      resolution: hasVideo
        ? f.width !== null && f.height !== null
          ? `${f.width}×${f.height}`
          : (resolution(f.height, null) ?? (f.note || "?"))
        : t("media.format.audio_only"),
      fps: hasVideo && f.fps !== null ? String(Math.round(f.fps)) : "",
      codec: codecs.join(" + "),
      bitrate:
        f.bitrate !== null && f.bitrate > 0
          ? t("media.format.kbps", { value: Math.round(f.bitrate) })
          : "",
      size: sizeText(f.size, f.sizeApproximate) ?? "",
      protocol: protocolName(f.protocol),
      video: hasVideo,
      audio: f.audioCodec !== null && !hasVideo,
    };
  });
}

/** The expression for streams chosen in the table: `137+140`, `137`, `140`. */
export function expressionFor(video: string | null, audio: string | null): string {
  return [video, audio].filter((id): id is string => !!id).join("+");
}

/** Whether a typed format expression can be sent (the backend checks again). */
export function expressionProblem(text: string): string | null {
  const s = text.trim();
  // eslint-disable-next-line no-control-regex -- refusing control characters is the point
  if (!s || s.length > 512 || /[\u0000-\u001f\u007f]/.test(s)) {
    return t("media.expression.problem");
  }
  return null;
}

/** Whether a chosen expression gives audio only (one audio-only format). */
export function expressionIsAudio(expression: string, formats: readonly MediaFormat[]): boolean {
  const ids = expression.split("+");
  return (
    ids.length === 1 &&
    formats.some((f) => f.id === ids[0] && f.videoCodec === null && f.audioCodec !== null)
  );
}

// The install sheet --------------------------------------------------------

const TOOL_NAMES: Record<MediaTool, string> = { ytDlp: "yt-dlp", ffmpeg: "FFmpeg" };

export function toolName(tool: MediaTool): string {
  return TOOL_NAMES[tool];
}

/** The plan's lines (`yt-dlp 2026.10.26 · 37 MiB`) and its total. */
export function installSummary(plan: readonly MediaToolSize[]): {
  lines: { tool: MediaTool; name: string; version: string; size: string }[];
  total: string;
} {
  return {
    lines: plan.map((p) => ({
      tool: p.tool,
      name: toolName(p.tool),
      version: p.version,
      size: formatBytes(p.size),
    })),
    total: formatBytes(plan.reduce((sum, p) => sum + p.size, 0)),
  };
}

const STEP_KEYS: Record<Exclude<MediaInstallStep, "downloading">, PlainMessageKey> = {
  checking: "media.install.step.checking",
  verifying: "media.install.step.verifying",
  unpacking: "media.install.step.unpacking",
  testing: "media.install.step.testing",
  done: "media.install.step.done",
};

/** One tool's install, as its line shows it. */
export interface InstallLine {
  tool: MediaTool;
  name: string;
  text: string;
  /** 0 to 1 while downloading; `null` otherwise (no bar, or indeterminate). */
  fraction: number | null;
  /** Its install has begun. */
  started: boolean;
  done: boolean;
}

/** The install's lines after `events`, one per tool of the plan. */
export function installLines(
  plan: readonly MediaToolSize[],
  events: readonly MediaInstallEvent[],
): InstallLine[] {
  const tools: MediaTool[] = plan.length ? plan.map((p) => p.tool) : ["ytDlp", "ffmpeg"];
  return tools.map((tool) => {
    const last = [...events].reverse().find((e) => e.tool === tool);
    const name = toolName(tool);
    if (!last)
      return {
        tool,
        name,
        text: t("media.install.step.waiting"),
        fraction: null,
        started: false,
        done: false,
      };
    if (last.step === "downloading") {
      const total = last.total ?? plan.find((p) => p.tool === tool)?.size ?? null;
      return {
        tool,
        name,
        text:
          total !== null
            ? t("media.install.step.downloading", {
                done: formatBytes(last.done),
                total: formatBytes(total),
              })
            : formatBytes(last.done),
        fraction: total ? Math.min(1, last.done / total) : null,
        started: true,
        done: false,
      };
    }
    return {
      tool,
      name,
      text: t(STEP_KEYS[last.step]),
      fraction: null,
      started: true,
      done: last.step === "done",
    };
  });
}
