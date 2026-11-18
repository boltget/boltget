import { describe, expect, it } from "vitest";
import type { MediaFormat, MediaHintInfo, MediaPreset, MediaSummary } from "../bindings";
import { formatBytes } from "../format";
import {
  expressionFor,
  expressionIsAudio,
  expressionProblem,
  failureHint,
  formatRows,
  hintLine,
  initialPreset,
  installLines,
  installSummary,
  linkResolution,
  mediaLength,
  offeredHints,
  pickerNotice,
  presetDetails,
  presetName,
  presetsFor,
  protocolName,
  requestContainer,
  resolution,
  sizeText,
  subtitleOptions,
} from "./picker";

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
    keptOriginal: false,
    ...over,
  };
}

function summary(over: Partial<MediaSummary> = {}): MediaSummary {
  return {
    url: "https://v.test/watch?v=1",
    site: "v.test",
    title: "A Video",
    uploader: "Chan",
    duration: 212,
    thumbnail: null,
    presets: [preset(), preset({ preset: "audio", audioOnly: true, height: null })],
    originalPresets: [preset({ container: "webm", videoCodec: "VP9", audioCodec: "Opus" })],
    formats: [],
    subtitles: [],
    isPlaylist: false,
    entryCount: 0,
    playlist: null,
    inPlaylist: false,
    drm: false,
    remembered: null,
    updated: false,
    ...over,
  };
}

function fmt(over: Partial<MediaFormat> = {}): MediaFormat {
  return {
    id: "137",
    ext: "mp4",
    width: 1920,
    height: 1080,
    fps: 30,
    videoCodec: "H.264",
    audioCodec: null,
    bitrate: 4400.4,
    size: 120 * MIB,
    sizeApproximate: false,
    protocol: "https",
    note: null,
    ...over,
  };
}

describe("sizes and lengths", () => {
  it("formats lengths", () => {
    expect(mediaLength(212)).toBe("3:32");
    expect(mediaLength(3725)).toBe("1:02:05");
    expect(mediaLength(-4)).toBe("0:00");
  });

  it("says about for estimates and hides unknown sizes", () => {
    expect(sizeText(31 * MIB, false)).toBe(formatBytes(31 * MIB));
    expect(sizeText(31 * MIB, true)).toBe(`about ${formatBytes(31 * MIB)}`);
    expect(sizeText(null, true)).toBeNull();
  });
});

describe("presets", () => {
  it("names presets", () => {
    expect(presetName("best")).toBe("Best quality");
    expect(presetName("small")).toBe("Smaller file");
    expect(presetName("audio")).toBe("Audio only");
    expect(presetName("1080p")).toBe("1080p");
    expect(presetName("strange")).toBe("strange");
  });

  it("shows resolution, container, codecs and size", () => {
    expect(presetDetails(preset())).toEqual(["1080p", "MP4", "AV1 + AAC", formatBytes(31 * MIB)]);
    expect(presetDetails(preset({ fps: 59.94, sizeApproximate: true }))).toEqual([
      "1080p60",
      "MP4",
      "AV1 + AAC",
      `about ${formatBytes(31 * MIB)}`,
    ]);
    // Audio only: no resolution or video codec; no size when unknown.
    expect(
      presetDetails(
        preset({ audioOnly: true, height: null, container: "m4a", videoCodec: null, size: null }),
      ),
    ).toEqual(["M4A", "AAC"]);
    expect(resolution(720, 30)).toBe("720p");
    expect(resolution(null, 60)).toBeNull();
  });

  it("follows the container toggle", () => {
    const m = summary();
    expect(presetsFor(m, "mp4")).toBe(m.presets);
    expect(presetsFor(m, "original")).toBe(m.originalPresets);
  });

  it("keeps the choice, else the site's last one, else the first", () => {
    const list = [preset(), preset({ preset: "720p" }), preset({ preset: "audio" })];
    expect(initialPreset(list, null, null)).toBe("best");
    expect(initialPreset(list, null, "720p")).toBe("720p");
    expect(initialPreset(list, "audio", "720p")).toBe("audio");
    // A preset collapsed into another one (1080p into Best quality with
    // Prefer MP4) is not offered: the first, which gives the same file.
    expect(initialPreset(list, "1080p", "1080p")).toBe("best");
    expect(initialPreset([], "best", null)).toBeNull();
  });

  it("says when there is nothing to choose", () => {
    expect(pickerNotice(summary())).toBeNull();
    expect(pickerNotice(summary({ isPlaylist: true, entryCount: 12 }))).toBe("playlist");
    expect(pickerNotice(summary({ drm: true, presets: [], originalPresets: [] }))).toBe("drm");
    expect(pickerNotice(summary({ presets: [], originalPresets: [] }))).toBe("none");
  });
});

describe("subtitles", () => {
  it("lists the site's own first, then automatic captions, marked", () => {
    const options = subtitleOptions([
      { code: "tr", name: "Turkish", automatic: true },
      { code: "en", name: "English", automatic: true },
      { code: "de", name: "German", automatic: false },
      { code: "en", name: "English", automatic: false },
      { code: "xx", name: null, automatic: false },
    ]);
    expect(options.map((o) => o.label)).toEqual([
      "English",
      "German",
      "xx",
      "English (automatic)",
      "Turkish (automatic)",
    ]);
    // English twice, with distinct values.
    expect(new Set(options.map((o) => o.value)).size).toBe(5);
    expect(options[3]).toMatchObject({ code: "en", automatic: true, value: "auto:en" });
  });
});

describe("the advanced table", () => {
  it("shows each format as text", () => {
    const [video, audio, combined, bare] = formatRows([
      fmt(),
      fmt({
        id: "140",
        width: null,
        height: null,
        fps: null,
        videoCodec: null,
        audioCodec: "AAC",
        bitrate: 129,
        size: 3 * MIB,
        sizeApproximate: true,
      }),
      fmt({ id: "96", audioCodec: "AAC", size: null, protocol: "m3u8_native" }),
      fmt({ id: "x", width: null, height: 480, fps: null, bitrate: null }),
    ]);
    expect(video).toMatchObject({
      id: "137",
      resolution: "1920×1080",
      fps: "30",
      codec: "H.264",
      bitrate: "4,400 kbit/s",
      size: formatBytes(120 * MIB),
      protocol: "HTTPS",
      video: true,
      audio: false,
    });
    expect(audio).toMatchObject({
      resolution: "Audio only",
      fps: "",
      codec: "AAC",
      size: `about ${formatBytes(3 * MIB)}`,
      video: false,
      audio: true,
    });
    // Video and audio together: a video stream (no separate audio needed).
    expect(combined).toMatchObject({
      codec: "H.264 + AAC",
      size: "",
      protocol: "HLS",
      video: true,
    });
    expect(bare).toMatchObject({ resolution: "480p", fps: "", bitrate: "" });
    expect(protocolName("http_dash_segments+https")).toBe("DASH + HTTPS");
    expect(protocolName(null)).toBe("");
  });

  it("builds and checks expressions", () => {
    expect(expressionFor("137", "140")).toBe("137+140");
    expect(expressionFor("137", null)).toBe("137");
    expect(expressionFor(null, "140")).toBe("140");
    expect(expressionFor(null, null)).toBe("");
    expect(expressionProblem("137+140")).toBeNull();
    expect(expressionProblem("  ")).not.toBeNull();
    expect(expressionProblem("a\nb")).not.toBeNull();
    expect(expressionProblem("x".repeat(513))).not.toBeNull();
    const formats = [fmt(), fmt({ id: "140", videoCodec: null, audioCodec: "AAC" })];
    expect(expressionIsAudio("140", formats)).toBe(true);
    expect(expressionIsAudio("137+140", formats)).toBe(false);
    expect(expressionIsAudio("bestaudio", formats)).toBe(false);
  });
});

describe("the install sheet", () => {
  const plan = [
    { tool: "ytDlp" as const, version: "2026.10.26", size: 38 * MIB },
    { tool: "ffmpeg" as const, version: "n9.0.2", size: 86 * MIB },
  ];

  it("lists the tools with their sizes and the total", () => {
    const s = installSummary(plan);
    expect(s.lines.map((l) => [l.name, l.version, l.size])).toEqual([
      ["yt-dlp", "2026.10.26", formatBytes(38 * MIB)],
      ["FFmpeg", "n9.0.2", formatBytes(86 * MIB)],
    ]);
    expect(s.total).toBe(formatBytes(124 * MIB));
  });

  it("follows the install's events per tool", () => {
    expect(installLines(plan, []).map((l) => l.text)).toEqual(["Waiting", "Waiting"]);
    const lines = installLines(plan, [
      { tool: "ytDlp", step: "checking", done: 0, total: null },
      { tool: "ytDlp", step: "downloading", done: 19 * MIB, total: 38 * MIB },
      { tool: "ytDlp", step: "done", done: 0, total: null },
      { tool: "ffmpeg", step: "downloading", done: 43 * MIB, total: null },
    ]);
    expect(lines[0]).toMatchObject({ text: "Ready", done: true, fraction: null });
    // The plan's size stands in for a total the download does not know.
    expect(lines[1]).toMatchObject({ done: false, fraction: 0.5 });
    expect(lines[1]?.text).toBe(`${formatBytes(43 * MIB)} of ${formatBytes(86 * MIB)}`);
    // Without a plan, both tools.
    expect(installLines([], []).map((l) => l.name)).toEqual(["yt-dlp", "FFmpeg"]);
  });
});

describe("hints from the browser", () => {
  const hls: MediaHintInfo = {
    url: "https://cdn.example.com/v/720p/index.m3u8?token=x",
    kind: "hls",
    mime: "application/vnd.apple.mpegurl",
    size: null,
  };
  const file: MediaHintInfo = {
    url: "https://media.example.org/clip_1920x1080.mp4",
    kind: "file",
    mime: "video/mp4",
    size: 48 * MIB,
  };

  it("are offered only when the page gave nothing to download", () => {
    const page = "https://www.example.com/watch";
    expect(offeredHints([hls, file], "failed", page)).toEqual([hls, file]);
    expect(offeredHints([hls], "none", page)).toEqual([hls]);
    expect(offeredHints([hls], null, page)).toEqual([]);
    expect(offeredHints([hls], "drm", page)).toEqual([]);
    expect(offeredHints([hls], "playlist", page)).toEqual([]);
    expect(offeredHints([], "failed", page)).toEqual([]);
    // A hint that failed leaves the others.
    expect(offeredHints([hls, file], "failed", hls.url)).toEqual([file]);
  });

  it("name what they are, the resolution in the link, the host and the size", () => {
    expect(hintLine(hls)).toEqual({
      name: "HLS stream",
      host: "cdn.example.com",
      details: ["720p", "cdn.example.com"],
    });
    expect(hintLine(file)).toEqual({
      name: "Video file",
      host: "media.example.org",
      details: ["1080p", "media.example.org", formatBytes(48 * MIB)],
    });
    expect(hintLine({ ...file, url: "https://a.test/s.m4a", mime: "audio/mp4" }).name).toBe(
      "Audio file",
    );
    expect(hintLine({ ...hls, kind: "dash", url: "https://a.test/m.mpd" })).toEqual({
      name: "DASH stream",
      host: "a.test",
      details: ["a.test"],
    });
  });

  it("read a resolution from the link only where one is named", () => {
    expect(linkResolution("https://a.test/hls/1080p60/x.m3u8")).toBe("1080p");
    expect(linkResolution("https://a.test/v_1280x720.mp4")).toBe("720p");
    expect(linkResolution("https://a.test/480p.mp4")).toBe("480p");
    expect(linkResolution("https://a.test/v/12345678.mp4")).toBeNull();
    expect(linkResolution("https://a.test/mp4p/a.mp4")).toBeNull();
    expect(linkResolution("not a link")).toBeNull();
  });
});

describe("failures and containers", () => {
  it("adds a hint where the message alone does not say what to do", () => {
    expect(failureHint("error.media.sign_in")).toBe("media.error.sign_in.hint");
    expect(failureHint("error.media.site_changed.custom")).toBe("media.error.custom.hint");
    for (const key of [
      "error.media.unavailable",
      "error.media.site_changed",
      "error.media.site_changed.current",
      "error.media.network",
      "error.media.other",
    ]) {
      expect(failureHint(key)).toBeNull();
    }
  });

  it("keeps the original container for streams MP4 cannot hold", () => {
    const webm = preset({ container: "webm", keptOriginal: true });
    expect(requestContainer("mp4", webm, false)).toBe("original");
    expect(requestContainer("mp4", preset(), false)).toBe("mp4");
    expect(requestContainer("original", preset(), false)).toBe("original");
    // A typed expression: the switch decides (the backend falls back).
    expect(requestContainer("mp4", webm, true)).toBe("mp4");
    expect(requestContainer("mp4", null, false)).toBe("mp4");
  });
});
