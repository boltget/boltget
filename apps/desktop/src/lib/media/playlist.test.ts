import { describe, expect, it } from "vitest";
import type { MediaEntry, MediaSummary } from "../bindings";
import {
  PLAYLIST_PRESETS,
  choosable,
  entryLabel,
  mergeEntries,
  moveActive,
  playlistPresetDetails,
  playlistRequest,
  playlistSubtitleOptions,
  selectAll,
  selectRange,
  selectionText,
  toggle,
} from "./playlist";

function entry(index: number, over: Partial<MediaEntry> = {}): MediaEntry {
  return {
    index,
    id: `v${index}`,
    title: `Part ${index}`,
    url: `https://v.test/watch?v=v${index}`,
    duration: 60 * index,
    uploader: null,
    thumbnail: null,
    available: true,
    ...over,
  };
}

// 1 to 8, with 3 private and 6 without a page.
const ENTRIES = [1, 2, 3, 4, 5, 6, 7, 8].map((i) =>
  i === 3
    ? entry(i, { title: "[Private video]", available: false, duration: null })
    : i === 6
      ? entry(i, { url: null })
      : entry(i),
);

const set = (...n: number[]) => new Set(n);

describe("choosing a playlist's entries", () => {
  it("skips private and deleted entries everywhere", () => {
    expect(choosable(ENTRIES)).toEqual([1, 2, 4, 5, 7, 8]);
    expect(selectAll(ENTRIES)).toEqual(set(1, 2, 4, 5, 7, 8));
    expect(toggle(set(), ENTRIES, 3)).toEqual(set());
    expect(toggle(set(1), ENTRIES, 1)).toEqual(set());
    expect(toggle(set(1), ENTRIES, 2)).toEqual(set(1, 2));
  });

  it("a Shift range takes the state of the entry it starts from, either way", () => {
    // From a chosen anchor: everything between is chosen (unavailable skipped).
    expect(selectRange(set(2), ENTRIES, 2, 7)).toEqual(set(2, 4, 5, 7));
    // Upwards too.
    expect(selectRange(set(7), ENTRIES, 7, 4)).toEqual(set(4, 5, 7));
    // From an anchor that is not chosen: the range is cleared.
    const cleared = toggle(selectAll(ENTRIES), ENTRIES, 4);
    expect(selectRange(cleared, ENTRIES, 4, 8)).toEqual(set(1, 2));
    // Without an anchor: chooses up to there.
    expect(selectRange(set(), ENTRIES, null, 2)).toEqual(set(2));
    // An unknown place changes nothing.
    expect(selectRange(set(1), ENTRIES, 1, 99)).toEqual(set(1));
  });

  it("moves through the list with the keyboard, clamped at the ends", () => {
    expect(moveActive(ENTRIES, null, 1)).toBe(1);
    expect(moveActive(ENTRIES, null, -1)).toBe(8);
    expect(moveActive(ENTRIES, 2, 1)).toBe(3);
    expect(moveActive(ENTRIES, 7, 6)).toBe(8);
    expect(moveActive(ENTRIES, 2, -8)).toBe(1);
    expect(moveActive([], 1, 1)).toBeNull();
  });

  it("counts what is chosen of what can be", () => {
    expect(selectionText(set(1, 2, 3), ENTRIES)).toBe("2 of 6 selected");
  });

  it("merges a further page without listing a place twice", () => {
    const more = [entry(8), entry(9), entry(10)];
    expect(mergeEntries(ENTRIES, more).map((e) => e.index)).toEqual([
      1, 2, 3, 4, 5, 6, 7, 8, 9, 10,
    ]);
  });

  it("names entries for screen readers", () => {
    expect(entryLabel(entry(7, { duration: 212 }))).toBe("7. Part 7, 3:32");
    expect(entryLabel(ENTRIES[2] as MediaEntry)).toBe("3. [Private video], Unavailable");
    expect(entryLabel(entry(1, { title: " ", duration: null }))).toBe("1. Untitled video");
  });
});

describe("one preset for all", () => {
  it("describes each preset without sizes", () => {
    expect(PLAYLIST_PRESETS).toEqual(["best", "1080p", "720p", "small", "audio"]);
    expect(playlistPresetDetails("best", "mp4")).toEqual(["The best each video has", "MP4"]);
    expect(playlistPresetDetails("720p", "original")).toEqual(["Up to 720p", "original format"]);
    expect(playlistPresetDetails("small", "mp4")).toEqual(["Up to 480p", "MP4"]);
    expect(playlistPresetDetails("audio", "mp4")).toEqual(["The audio alone", "M4A"]);
  });

  it("offers common subtitle languages, each if available", () => {
    const options = playlistSubtitleOptions();
    expect(options[0]).toEqual({ value: "en", label: "English, if available" });
    expect(new Set(options.map((o) => o.value)).size).toBe(options.length);
  });

  it("asks for the chosen entries in the playlist's order", () => {
    const media = {
      url: "https://v.test/playlist?list=PL1",
      title: "Weekend",
      uploader: "Chan",
      entryCount: 8,
      playlist: { id: "PL1", start: 1, entries: ENTRIES, more: false, folder: "Weekend" },
    } as MediaSummary;
    const request = playlistRequest(media, ENTRIES, set(7, 1, 3, 6), {
      preset: "audio",
      container: "mp4",
      subtitle: "en",
      embed: true,
      page: 4,
    });
    expect(request.entries).toEqual([
      { index: 1, url: "https://v.test/watch?v=v1", title: "Part 1" },
      { index: 7, url: "https://v.test/watch?v=v7", title: "Part 7" },
    ]);
    expect(request.id).toBe("PL1");
    expect(request.entryCount).toBe(8);
    expect(request.page).toBe(4);
    // Audio files cannot carry subtitles: they are saved next to it.
    expect(request.subtitles).toEqual({ languages: ["en"], automatic: true, embed: false });
    expect(
      playlistRequest(media, ENTRIES, set(1), {
        preset: "best",
        container: "original",
        subtitle: null,
        embed: true,
        page: null,
      }).subtitles,
    ).toBeNull();
  });
});
