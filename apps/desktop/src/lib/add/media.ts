// "Look for media on this page" in the add sheet (docs/VIDEO.md): what the
// sheet shows for each answer of `probeMedia`. The picker itself arrives in
// v0.4 step 3; this is the compact result.
import { isPlainMessageKey, t } from "@boltget/ui";
import type { MediaPreset, MediaProbe, UiError } from "../bindings";
import { formatBytes } from "../format";

/** Where a look for media stands. */
export type MediaState =
  | { status: "looking" }
  | { status: "done"; result: MediaProbe }
  | { status: "failed"; error: UiError };

/** What the sheet shows for it. */
export type MediaView =
  | { kind: "looking" }
  | { kind: "message"; text: string; tone: "info" | "warning" }
  | { kind: "media"; title: string; meta: string[]; best: string[] };

/** A video's length: `3:32`, `1:02:05`. */
export function mediaLength(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  const two = (n: number) => String(n).padStart(2, "0");
  const h = Math.floor(s / 3600);
  const m = Math.floor(s / 60) % 60;
  return h > 0 ? `${h}:${two(m)}:${two(s % 60)}` : `${m}:${two(s % 60)}`;
}

/** A preset in a few words: `1080p · MP4 · H.264 + AAC · about 31 MiB`. */
export function presetSummary(p: MediaPreset): string[] {
  const parts = [
    p.height !== null ? `${p.height}p` : t("add.media.audio"),
    p.container.toUpperCase(),
  ];
  const codecs = [p.videoCodec, p.audioCodec].filter((c): c is string => !!c);
  if (codecs.length) parts.push(codecs.join(" + "));
  if (p.size !== null) {
    const size = formatBytes(p.size);
    parts.push(p.sizeApproximate ? t("add.media.about", { size }) : size);
  }
  return parts;
}

export function mediaView(state: MediaState): MediaView {
  if (state.status === "looking") return { kind: "looking" };
  if (state.status === "failed") {
    const { key } = state.error;
    // yt-dlp's own failure means, for now, that it found nothing it can use;
    // step 6 of v0.4 tells its reasons apart.
    const text =
      key === "error.media.extraction"
        ? t("add.media.none")
        : isPlainMessageKey(key)
          ? t(key)
          : t("error.unknown");
    return { kind: "message", text, tone: "warning" };
  }
  const { result } = state;
  if (result.kind === "notInstalled") {
    return { kind: "message", text: t("add.media.not_installed"), tone: "info" };
  }
  const m = result.media;
  if (m.drm) return { kind: "message", text: t("add.media.drm"), tone: "warning" };
  const meta = [
    ...(m.uploader ? [m.uploader] : []),
    ...(m.duration !== null ? [mediaLength(m.duration)] : []),
  ];
  if (m.isPlaylist) {
    return {
      kind: "media",
      title: m.title ?? "",
      meta: [...meta, t("add.media.playlist", { count: m.entryCount })],
      best: [],
    };
  }
  const best = m.presets[0];
  if (!best) return { kind: "message", text: t("add.media.none"), tone: "info" };
  return { kind: "media", title: m.title ?? "", meta, best: presetSummary(best) };
}
