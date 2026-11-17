// Playlists in the list (docs/VIDEO.md, "Playlists: as built"): each
// chosen entry of a playlist is a media download of its own, and the
// entries of one playlist run one at a time, as a queue of their own. The
// sidebar lists playlists with entries still to download; a playlist's view
// shows its entries in its order. Pure rules over the rows' values.
import type { DownloadState } from "../bindings";

/** What the sidebar and a playlist's view need of a row. */
export interface PlaylistRow {
  state: DownloadState;
  media: { playlist: { id: number; title: string | null } | null } | null;
}

/** A playlist as the list knows it, from its entries' rows. */
export interface PlaylistInfo {
  id: number;
  title: string | null;
  /** Entries in the list. */
  total: number;
  /** Entries done. */
  done: number;
  /** Entries running or waiting their turn. */
  active: number;
  /** Entries paused or failed (Resume all takes them). */
  stopped: number;
}

const RUNNING: ReadonlySet<DownloadState> = new Set(["queued", "downloading", "processing"]);

/** Every playlist the rows belong to, oldest first. */
export function playlistsOf(rows: readonly PlaylistRow[]): PlaylistInfo[] {
  const map = new Map<number, PlaylistInfo>();
  for (const row of rows) {
    const p = row.media?.playlist;
    if (!p) continue;
    let info = map.get(p.id);
    if (!info) {
      info = { id: p.id, title: p.title, total: 0, done: 0, active: 0, stopped: 0 };
      map.set(p.id, info);
    }
    info.total += 1;
    if (row.state === "completed") info.done += 1;
    else if (RUNNING.has(row.state)) info.active += 1;
    else info.stopped += 1;
  }
  return [...map.values()].sort((a, b) => a.id - b.id);
}

/**
 * The playlists the sidebar lists: those with entries still to download,
 * and the one whose view is open (so it does not vanish under the user).
 */
export function sidebarPlaylists(
  all: readonly PlaylistInfo[],
  current: number | null,
): PlaylistInfo[] {
  return all.filter((p) => p.done < p.total || p.id === current);
}
