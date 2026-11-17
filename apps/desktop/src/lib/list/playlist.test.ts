import { describe, expect, it } from "vitest";
import type { DownloadState } from "../bindings";
import { buildLayout, inView, playlistOfView, playlistView, type ListItem } from "./model";
import { playlistsOf, sidebarPlaylists } from "./playlist";

function row(state: DownloadState, playlist: number | null, title = "List") {
  return { state, media: playlist === null ? null : { playlist: { id: playlist, title } } };
}

function item(id: number, playlist: number | null, index: number | null): ListItem {
  return {
    id,
    state: "queued",
    name: `v${id}.mp4`,
    url: "https://v.test/w",
    total: null,
    createdAt: id,
    completedAt: null,
    queue: 0,
    queuePosition: Number.MAX_SAFE_INTEGER,
    startAt: null,
    playlist,
    playlistIndex: index,
  };
}

describe("playlists in the list", () => {
  it("are summed up from their entries' rows", () => {
    const list = playlistsOf([
      row("completed", 2, "B"),
      row("downloading", 1, "A"),
      row("queued", 1, "A"),
      row("paused", 1, "A"),
      row("failed", 2, "B"),
      row("completed", null),
    ]);
    expect(list).toEqual([
      { id: 1, title: "A", total: 3, done: 0, active: 2, stopped: 1 },
      { id: 2, title: "B", total: 2, done: 1, active: 0, stopped: 1 },
    ]);
  });

  it("leave the sidebar when done, unless their view is open", () => {
    const list = playlistsOf([row("completed", 1), row("queued", 2)]);
    expect(sidebarPlaylists(list, null).map((p) => p.id)).toEqual([2]);
    expect(sidebarPlaylists(list, 1).map((p) => p.id)).toEqual([1, 2]);
  });

  it("have a view with their entries in their order, as one flat list", () => {
    const view = playlistView(7);
    expect(playlistOfView(view)).toBe(7);
    expect(playlistOfView("all")).toBeNull();
    const items = [item(1, 7, 3), item(2, 7, 1), item(3, null, null), item(4, 8, 2)];
    expect(items.filter((i) => inView(i, view)).map((i) => i.id)).toEqual([1, 2]);
    const layout = buildLayout(items, { view, query: "", sort: "default", now: 0 });
    expect(layout.order).toEqual([2, 1]);
    expect(layout.entries.every((e) => e.kind === "row")).toBe(true);
  });
});
