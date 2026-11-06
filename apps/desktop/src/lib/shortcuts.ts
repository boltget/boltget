// The window's keyboard shortcuts (DESIGN-PRINCIPLES #6, APP-DESIGN #20),
// in one list: Settings shows it, and the palette shows each action's keys.
// The keys themselves are handled where they act (App.svelte, the list, the
// title bar); this list is what the user is told.
import type { PlainMessageKey } from "@boltget/ui/messages";

export type ShortcutId =
  | "palette"
  | "add"
  | "paste"
  | "search"
  | "settings"
  | "pauseAll"
  | "resumeAll"
  | "toggle"
  | "open"
  | "show"
  | "copy"
  | "remove"
  | "delete"
  | "selectAll"
  | "menu"
  | "details"
  | "density"
  | "move";

export interface Shortcut {
  id: ShortcutId;
  /** The keys, as keycap text ("Ctrl+Shift+P"). */
  keys: PlainMessageKey;
  /** What it does. */
  label: PlainMessageKey;
}

export const SHORTCUTS: readonly Shortcut[] = [
  { id: "palette", keys: "shortcut.palette", label: "shortcuts.palette" },
  { id: "add", keys: "shortcut.add", label: "command.add" },
  { id: "paste", keys: "shortcut.paste", label: "shortcuts.paste" },
  { id: "search", keys: "shortcut.search", label: "command.search" },
  { id: "settings", keys: "shortcut.settings", label: "command.settings" },
  { id: "pauseAll", keys: "shortcut.pause_all", label: "command.pause_all" },
  { id: "resumeAll", keys: "shortcut.resume_all", label: "command.resume_all" },
  { id: "toggle", keys: "shortcut.toggle", label: "shortcuts.toggle" },
  { id: "open", keys: "shortcut.enter", label: "shortcuts.open" },
  { id: "show", keys: "shortcut.show", label: "shortcuts.show" },
  { id: "copy", keys: "shortcut.copy", label: "shortcuts.copy" },
  { id: "remove", keys: "shortcut.remove", label: "shortcuts.remove" },
  { id: "delete", keys: "shortcut.delete", label: "shortcuts.delete" },
  { id: "selectAll", keys: "shortcut.select_all", label: "shortcuts.select_all" },
  { id: "menu", keys: "shortcut.menu", label: "shortcuts.menu" },
  { id: "details", keys: "shortcut.details", label: "command.details" },
  { id: "density", keys: "shortcut.density", label: "density.compact" },
  { id: "move", keys: "shortcut.move", label: "shortcuts.move" },
];

const BY_ID = new Map(SHORTCUTS.map((s) => [s.id, s]));

/** The keycap text of a shortcut. */
export function keysOf(id: ShortcutId): PlainMessageKey {
  return (BY_ID.get(id) as Shortcut).keys;
}

/** Whether a key event is `Ctrl` (or `Cmd`) plus exactly these modifiers. */
export function withCtrl(event: KeyboardEvent, shift = false): boolean {
  return (event.ctrlKey || event.metaKey) && !event.altKey && event.shiftKey === shift;
}
