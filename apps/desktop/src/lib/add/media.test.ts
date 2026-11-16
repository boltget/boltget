import { describe, expect, it } from "vitest";
import type { MediaPreset, MediaSummary } from "../bindings";
import { formatBytes } from "../format";
import { mediaLength, mediaView, presetSummary } from "./media";

const MIB = 1024 * 1024;

function preset(over: Partial<MediaPreset> = {}): MediaPreset {
  return {
    preset: "best",
    expression: "bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/bv*+ba/b",
    audioOnly: false,
    width: 1920,
    height: 1080,
    fps: 25,
    container: "mp4",
    videoCodec: "AV1",
    audioCodec: "AAC",
    size: 31 * MIB,
    sizeApproximate: false,
    ...over,
  };
}

function summary(over: Partial<MediaSummary> = {}): MediaSummary {
  return {
    title: "A Video",
    uploader: "Chan",
    duration: 212,
    thumbnail: null,
    presets: [preset()],
    subtitles: [],
    isPlaylist: false,
    entryCount: 0,
    drm: false,
    ...over,
  };
}

const found = (media: MediaSummary) =>
  mediaView({ status: "done", result: { kind: "found", media } });

describe("media in the add sheet", () => {
  it("formats lengths like a player", () => {
    expect(mediaLength(212)).toBe("3:32");
    expect(mediaLength(3725.4)).toBe("1:02:05");
    expect(mediaLength(-3)).toBe("0:00");
  });

  it("sums a preset up", () => {
    expect(presetSummary(preset())).toEqual(["1080p", "MP4", "AV1 + AAC", formatBytes(31 * MIB)]);
    expect(
      presetSummary(
        preset({ height: null, container: "m4a", videoCodec: null, sizeApproximate: true }),
      ),
    ).toEqual(["Audio only", "M4A", "AAC", `about ${formatBytes(31 * MIB)}`]);
    expect(presetSummary(preset({ size: null, videoCodec: null, audioCodec: null }))).toEqual([
      "1080p",
      "MP4",
    ]);
  });

  it("shows the best preset of a video", () => {
    expect(found(summary())).toEqual({
      kind: "media",
      title: "A Video",
      meta: ["Chan", "3:32"],
      best: ["1080p", "MP4", "AV1 + AAC", formatBytes(31 * MIB)],
    });
  });

  it("says when there is nothing to download", () => {
    expect(found(summary({ presets: [] }))).toMatchObject({ kind: "message", tone: "info" });
    expect(found(summary({ presets: [], drm: true }))).toMatchObject({
      kind: "message",
      tone: "warning",
      text: "This is protected by DRM, so Boltget cannot download it.",
    });
    expect(
      mediaView({
        status: "failed",
        error: { key: "error.media.extraction", message: "Unsupported URL" },
      }),
    ).toEqual({ kind: "message", tone: "warning", text: "No video or audio found on this page." });
    expect(
      mediaView({ status: "failed", error: { key: "error.media.timeout", message: null } }),
    ).toMatchObject({ text: "The video tool took too long and was stopped." });
    expect(mediaView({ status: "done", result: { kind: "notInstalled" } })).toEqual({
      kind: "message",
      tone: "info",
      text: "Video tools are not installed yet.",
    });
  });

  it("counts a playlist's videos", () => {
    const view = found(summary({ presets: [], isPlaylist: true, entryCount: 12, duration: null }));
    expect(view).toEqual({
      kind: "media",
      title: "A Video",
      meta: ["Chan", "A playlist with 12 videos"],
      best: [],
    });
    expect(mediaView({ status: "looking" })).toEqual({ kind: "looking" });
  });
});
