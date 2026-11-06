// The window's own state, outside the engine: which screen (the downloads
// or Settings), view and search, the selection, density, the details pane,
// the command palette, quiet notices (toasts), the one confirmation the app
// asks for, and the actions every surface shares (row buttons, the context
// menu, shortcuts, the palette and the details pane).
import { t } from "@boltget/ui/messages";
import type { DownloadId } from "./bindings";
import { downloads, errorText } from "./engine";
import { canPause, canRestart, canResume, toggleIntent, type ActionId } from "./list/actions";
import type { Sort, View } from "./list/model";
import type { RowId, SectionId } from "./settings/registry";
import { sectionOf } from "./settings/registry";
import { EMPTY, type Selection } from "./list/selection";
import { displayName } from "./format";

export type Density = "comfortable" | "compact";
export type Screen = "downloads" | "settings";

export interface Toast {
  id: number;
  text: string;
  action?: { label: string; run: () => void };
}

export interface Confirmation {
  title: string;
  body: string;
  confirm: string;
  run: () => void;
}

/** How long "Removed … Undo" stays before the removal is made. */
export const UNDO_MS = 6000;
const TOAST_MS = 4000;

function stored<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const value = localStorage.getItem(key);
    return allowed.includes(value as T) ? (value as T) : fallback;
  } catch {
    return fallback;
  }
}

function store(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Storage is a convenience; without it the setting lasts this session.
  }
}

class AppState {
  screen = $state<Screen>("downloads");
  /** The Settings section on show, and its search. */
  settingsSection = $state<SectionId>("general");
  settingsQuery = $state("");
  /** A setting to scroll to and focus once Settings shows it. */
  settingsTarget = $state<{ row: RowId; key: number } | null>(null);
  /** Whether the command palette is open. */
  palette = $state(false);
  view = $state<View>("all");
  query = $state("");
  sort = $state<Sort>("default");
  density = $state<Density>(stored("boltget.density", ["comfortable", "compact"], "comfortable"));
  paneOpen = $state(stored("boltget.pane", ["open", "closed"], "open") === "open");
  selection = $state.raw<Selection>(EMPTY);
  /** Rows removed with Undo still possible: hidden, not yet gone. */
  hidden = $state.raw<ReadonlySet<DownloadId>>(new Set());
  toasts = $state<Toast[]>([]);
  confirmation = $state<Confirmation | null>(null);
  /** Polite announcements for screen readers (completions, failures). */
  announcement = $state("");
  /** The download whose "Refresh link" field should take focus. */
  refreshing = $state<DownloadId | null>(null);
  /**
   * The add sheet (APP-DESIGN #6) while it is open: the links it was opened
   * with, and a number that changes when more arrive while it is open.
   */
  adding = $state<{ text: string; key: number } | null>(null);
  #toastId = 0;
  #addKey = 0;
  #targetKey = 0;

  /** Shows Settings, at a setting if one is named (the palette, a link). */
  openSettings(row?: RowId): void {
    this.screen = "settings";
    if (row) {
      this.settingsQuery = "";
      this.settingsSection = sectionOf(row);
      this.settingsTarget = { row, key: ++this.#targetKey };
    }
  }

  /** Back to the downloads. */
  closeSettings(): void {
    this.screen = "downloads";
    this.settingsQuery = "";
  }

  /** Shows a view of the list (the sidebar, the palette). */
  showView(view: View): void {
    this.closeSettings();
    this.view = view;
  }

  /** Opens the add sheet with `text` in its link field (or adds it there). */
  openAdd(text = ""): void {
    this.adding = { text, key: ++this.#addKey };
  }

  closeAdd(): void {
    this.adding = null;
  }

  /** Selects downloads and shows the first in the details pane. */
  select(ids: readonly DownloadId[]): void {
    const first = ids[0];
    if (first === undefined) return;
    this.selection = { ids: new Set(ids), anchor: first, active: first };
  }

  setDensity(density: Density): void {
    this.density = density;
    store("boltget.density", density);
  }

  togglePane(open = !this.paneOpen): void {
    this.paneOpen = open;
    store("boltget.pane", open ? "open" : "closed");
  }

  /** Selects a download and shows it, wherever it is: every one is in "All". */
  reveal(id: DownloadId): void {
    this.closeSettings();
    if (!downloads.get(id)) return;
    this.view = "all";
    this.query = "";
    this.select([id]);
    this.togglePane(true);
  }

  toast(text: string, action?: Toast["action"], ms = TOAST_MS): number {
    const id = ++this.#toastId;
    this.toasts = [...this.toasts.slice(-2), { id, text, action }];
    setTimeout(() => this.dismiss(id), ms);
    return id;
  }

  dismiss(id: number): void {
    this.toasts = this.toasts.filter((toast) => toast.id !== id);
  }

  announce(text: string): void {
    // A change of text is what screen readers announce; clear first so the
    // same sentence twice is still read.
    this.announcement = "";
    queueMicrotask(() => (this.announcement = text));
  }

  // Actions -----------------------------------------------------------------

  /** Runs an action on downloads (the selection, usually). */
  run(action: ActionId, ids: readonly DownloadId[]): void {
    const items = ids.flatMap((id) => downloads.get(id) ?? []);
    if (items.length === 0) return;
    const first = items[0];
    switch (action) {
      case "pause":
        for (const d of items) if (canPause(d.state)) void downloads.pause(d.id);
        break;
      case "resume":
      case "retry":
        for (const d of items) if (canResume(d.state)) void downloads.resume(d.id);
        break;
      case "restart":
        for (const d of items) if (canRestart(d.state)) void downloads.resume(d.id, true);
        break;
      case "refresh":
        if (first) {
          this.selection = { ids: new Set([first.id]), anchor: first.id, active: first.id };
          this.togglePane(true);
          this.refreshing = first.id;
        }
        break;
      case "open":
        if (first?.state === "completed") void downloads.open(first.id);
        break;
      case "show":
        if (first) void downloads.showInFolder(first.id);
        break;
      case "copy":
        void this.#copy(items.map((d) => d.url).join("\n"), items.length);
        break;
      case "remove":
        this.#removeWithUndo(items.map((d) => d.id));
        break;
      case "delete":
        this.#confirmDelete(items.map((d) => d.id));
        break;
    }
  }

  /** Space: pause what runs, else resume what stopped. */
  toggle(ids: readonly DownloadId[]): void {
    const states = ids.flatMap((id) => downloads.get(id)?.state ?? []);
    const intent = toggleIntent(states);
    if (intent) this.run(intent, ids);
  }

  pauseAll(): void {
    this.run(
      "pause",
      downloads.downloads.filter((d) => canPause(d.state)).map((d) => d.id),
    );
  }

  resumeAll(): void {
    this.run(
      "resume",
      downloads.downloads.filter((d) => d.state === "paused").map((d) => d.id),
    );
  }

  async #copy(text: string, count: number): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      this.toast(t("toast.copied", { count }));
    } catch {
      this.toast(t("error.clipboard"));
    }
  }

  #removeWithUndo(ids: DownloadId[]): void {
    if (ids.length === 0) return;
    const name = this.#nameOf(ids[0]);
    // A removed download stops at once; Undo starts it again.
    const running = ids.filter((id) => {
      const state = downloads.get(id)?.state;
      return state !== undefined && canPause(state);
    });
    for (const id of running) void downloads.pause(id);
    this.hidden = new Set([...this.hidden, ...ids]);
    let undone = false;
    const unhide = () => {
      // eslint-disable-next-line svelte/prefer-svelte-reactivity -- replaced whole, not mutated
      const next = new Set(this.hidden);
      for (const id of ids) next.delete(id);
      this.hidden = next;
    };
    const toast = this.toast(
      t("toast.removed", { count: ids.length, name }),
      {
        label: t("action.undo"),
        run: () => {
          undone = true;
          unhide();
          this.dismiss(toast);
          for (const id of running) void downloads.resume(id);
        },
      },
      UNDO_MS,
    );
    setTimeout(() => {
      if (undone) return;
      for (const id of ids) void downloads.remove(id, false);
      unhide();
    }, UNDO_MS);
  }

  #confirmDelete(ids: DownloadId[]): void {
    if (ids.length === 0) return;
    const name = this.#nameOf(ids[0]);
    this.confirmation = {
      title: t("confirm.delete.title", { count: ids.length, name }),
      body: t("confirm.delete.body", { count: ids.length }),
      confirm: t("action.delete"),
      run: () => {
        for (const id of ids) void downloads.remove(id, true);
        this.toast(t("toast.deleted", { count: ids.length, name }));
      },
    };
  }

  #nameOf(id: DownloadId | undefined): string {
    const d = id === undefined ? undefined : downloads.get(id);
    return d ? displayName(d.fileName, d.url) : "";
  }

  /** Shows the store's last failed action as a quiet notice. */
  reportErrors(): () => void {
    let last = downloads.lastError;
    return $effect.root(() => {
      $effect(() => {
        const error = downloads.lastError;
        if (error && error !== last) this.toast(errorText(error));
        last = error;
      });
    });
  }
}

export const app = new AppState();
