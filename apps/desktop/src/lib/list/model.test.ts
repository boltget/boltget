import { describe, expect, it } from "vitest";
import type { DownloadState } from "../bindings";
import { categoryOf } from "./category";
import {
  buildLayout,
  entryAt,
  inView,
  offsets,
  scrollToReveal,
  sectionOf,
  viewCounts,
  visibleRange,
  type BuildOptions,
  type ListItem,
} from "./model";

const NOW = new Date(2026, 10, 1, 15, 0).getTime();
const HOUR = 3_600_000;

function item(id: number, state: DownloadState, extra: Partial<ListItem> = {}): ListItem {
  return {
    id,
    state,
    name: `file-${id}.bin`,
    url: `https://example.com/file-${id}.bin`,
    total: id * 100,
    createdAt: NOW - id * HOUR,
    completedAt: state === "completed" ? NOW - id * HOUR : null,
    ...extra,
  };
}

const options: BuildOptions = { view: "all", query: "", sort: "default", now: NOW };

describe("categories", () => {
  it("come from the extension", () => {
    expect(categoryOf("ubuntu.iso")).toBe("archives");
    expect(categoryOf("Movie.MKV")).toBe("video");
    expect(categoryOf("setup.exe")).toBe("programs");
    expect(categoryOf("song.flac")).toBe("music");
    expect(categoryOf("report.pdf")).toBe("documents");
    expect(categoryOf("data.tar.zst")).toBe("archives");
    expect(categoryOf("README")).toBe("other");
    expect(categoryOf(".hidden")).toBe("other");
    expect(categoryOf("trailing.")).toBe("other");
  });
});

describe("sections", () => {
  it("put running and queued first, stopped next, then history by day", () => {
    expect(sectionOf(item(1, "downloading"), NOW)).toBe("active");
    expect(sectionOf(item(1, "queued"), NOW)).toBe("active");
    expect(sectionOf(item(1, "paused"), NOW)).toBe("waiting");
    expect(sectionOf(item(1, "failed"), NOW)).toBe("waiting");
    expect(sectionOf(item(1, "completed"), NOW)).toBe("today");
    expect(sectionOf(item(20, "completed"), NOW)).toBe("yesterday");
    expect(sectionOf(item(24 * 4, "completed"), NOW)).toBe("week");
    expect(sectionOf(item(24 * 30, "completed"), NOW)).toBe("earlier");
  });

  it("group the list with a header each, in order", () => {
    const items = [
      item(1, "completed"),
      item(2, "downloading"),
      item(3, "paused"),
      item(4, "failed"),
      item(30, "completed"),
    ];
    const { entries, order } = buildLayout(items, options);
    expect(entries.map((e) => (e.kind === "section" ? e.section : e.id))).toEqual([
      "active",
      2,
      "waiting",
      4, // needs attention first
      3,
      "today",
      1,
      "yesterday",
      30,
    ]);
    expect(order).toEqual([2, 4, 3, 1, 30]);
  });

  it("hold a row's section while the list is in use", () => {
    const before = buildLayout([item(1, "downloading"), item(2, "paused")], options);
    const after = buildLayout([item(1, "completed"), item(2, "paused")], {
      ...options,
      held: before.sections,
    });
    expect(after.order).toEqual([1, 2]);
    expect(after.sections.get(1)).toBe("active");
    const released = buildLayout([item(1, "completed"), item(2, "paused")], options);
    expect(released.order).toEqual([2, 1]);
  });
});

describe("views, search and sorting", () => {
  const items = [
    item(1, "downloading", { name: "movie.mkv" }),
    item(2, "completed", { name: "Setup.exe", total: 50 }),
    item(3, "paused", { name: "notes.pdf", total: 900 }),
    item(4, "queued", { name: "album.flac" }),
  ];

  it("filter by view and category", () => {
    expect(items.filter((i) => inView(i, "active")).map((i) => i.id)).toEqual([1, 4]);
    expect(items.filter((i) => inView(i, "completed")).map((i) => i.id)).toEqual([2]);
    expect(items.filter((i) => inView(i, "scheduled"))).toEqual([]);
    expect(items.filter((i) => inView(i, "video")).map((i) => i.id)).toEqual([1]);
    expect(viewCounts(items)).toEqual({ all: 4, active: 2, completed: 1, scheduled: 0 });
  });

  it("search names and links, ignoring case", () => {
    expect(buildLayout(items, { ...options, query: "SETUP" }).order).toEqual([2]);
    expect(buildLayout(items, { ...options, query: "file-3" }).order).toEqual([3]);
    expect(buildLayout(items, { ...options, query: "  " }).order).toHaveLength(4);
  });

  it("sort on request into one flat list", () => {
    const byName = buildLayout(items, { ...options, sort: "name" });
    expect(byName.entries.every((e) => e.kind === "row")).toBe(true);
    expect(byName.order).toEqual([4, 1, 3, 2]);
    expect(buildLayout(items, { ...options, sort: "size" }).order).toEqual([3, 4, 1, 2]);
    expect(buildLayout(items, { ...options, sort: "added" }).order).toEqual([1, 2, 3, 4]);
  });
});

describe("virtual scrolling", () => {
  const { entries } = buildLayout(
    Array.from({ length: 500 }, (_, i) => item(i + 1, i < 20 ? "downloading" : "completed")),
    options,
  );
  const tops = offsets(entries, 40, 36);

  it("adds up fixed heights", () => {
    expect(tops[0]).toBe(0);
    expect(tops[1]).toBe(36);
    expect(tops[2]).toBe(76);
    expect(tops[entries.length]).toBe(
      500 * 40 + 36 * entries.filter((e) => e.kind === "section").length,
    );
  });

  it("finds the entry at a position", () => {
    expect(entryAt(tops, 0)).toBe(0);
    expect(entryAt(tops, 35)).toBe(0);
    expect(entryAt(tops, 36)).toBe(1);
    expect(entryAt(tops, 1e9)).toBe(entries.length - 1);
  });

  it("renders only what is visible, with overscan", () => {
    const [first, last] = visibleRange(tops, 4000, 800, 6);
    expect(last - first).toBeLessThan(40);
    expect(first).toBeGreaterThan(80);
    expect(visibleRange(offsets([], 40, 36), 0, 800)).toEqual([0, 0]);
  });

  it("scrolls just enough to reveal an entry", () => {
    expect(scrollToReveal(tops, 2, 0, 400)).toBe(0);
    expect(scrollToReveal(tops, 2, 200, 400)).toBe(76);
    expect(scrollToReveal(tops, 2, 200, 400, 36)).toBe(40);
    const index = 30;
    expect(scrollToReveal(tops, index, 0, 400)).toBe((tops[index + 1] ?? 0) - 400);
  });
});
