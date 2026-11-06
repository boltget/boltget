// Native context menus (APP-DESIGN #29): drawn by Windows through Tauri's
// `Menu.popup`, so they look, animate and read like every other menu on the
// system. Outside Tauri (the UI alone in a browser) nothing opens.
import { isTauri } from "@tauri-apps/api/core";
import { LogicalPosition } from "@tauri-apps/api/dpi";
import { Menu } from "@tauri-apps/api/menu";

export type MenuEntry =
  | {
      text: string;
      /** Shown at the right, e.g. `Ctrl+Enter`; display only. */
      accelerator?: string;
      enabled?: boolean;
      checked?: boolean;
      run: () => void;
    }
  | {
      text: string;
      enabled?: boolean;
      /** A submenu ("Move to queue ▸"). */
      items: MenuEntry[];
    }
  | "separator";

type NativeItem =
  | { item: "Separator" }
  | {
      id: string;
      text: string;
      enabled: boolean;
      accelerator?: string;
      checked?: boolean;
      action: () => void;
    }
  | { id: string; text: string; enabled: boolean; items: NativeItem[] };

function native(entries: MenuEntry[], prefix: string): NativeItem[] {
  return entries.map((entry, i): NativeItem => {
    if (entry === "separator") return { item: "Separator" };
    const id = `${prefix}${i}`;
    if ("items" in entry) {
      return {
        id,
        text: entry.text,
        enabled: entry.enabled ?? entry.items.length > 0,
        items: native(entry.items, `${id}.`),
      };
    }
    const base = {
      id,
      text: entry.text,
      enabled: entry.enabled ?? true,
      accelerator: entry.accelerator,
      action: () => entry.run(),
    };
    return entry.checked === undefined ? base : { ...base, checked: entry.checked };
  });
}

/** Shows a menu at the pointer, or at `at` (CSS pixels from the window's top left). */
export async function popupMenu(
  entries: MenuEntry[],
  at?: { x: number; y: number },
): Promise<void> {
  if (!isTauri()) return;
  const menu = await Menu.new({ items: native(entries, "m") });
  await menu.popup(at ? new LogicalPosition(at.x, at.y) : undefined);
}
