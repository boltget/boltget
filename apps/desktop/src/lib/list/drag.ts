// Dragging downloads inside the window: rows of a queue view onto another
// place in the queue, and rows of any view onto a queue in the sidebar.
// The ids travel as a type of their own, so the window's handler for
// dropped links (App.svelte) never mistakes them for a link to add.
import type { DownloadId } from "../bindings";

export const DOWNLOAD_IDS = "application/x-boltget-ids";

/** Starts a drag of these downloads. */
export function setDragIds(data: DataTransfer, ids: readonly DownloadId[]): void {
  data.setData(DOWNLOAD_IDS, ids.join(","));
  data.effectAllowed = "move";
}

/** Whether a drag carries downloads (readable while dragging over). */
export function isDownloadDrag(data: DataTransfer | null): boolean {
  return !!data && [...data.types].includes(DOWNLOAD_IDS);
}

/** The dragged downloads, on drop. */
export function dragIds(data: DataTransfer | null): DownloadId[] {
  const raw = data?.getData(DOWNLOAD_IDS) ?? "";
  return raw
    .split(",")
    .map(Number)
    .filter((n) => Number.isInteger(n) && n > 0);
}
