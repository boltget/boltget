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
  | "separator";

/** Shows a menu at the pointer, or at `at` (CSS pixels from the window's top left). */
export async function popupMenu(
  entries: MenuEntry[],
  at?: { x: number; y: number },
): Promise<void> {
  if (!isTauri()) return;
  const items = entries.map((entry, i) => {
    if (entry === "separator") return { item: "Separator" as const };
    const base = {
      id: `m${i}`,
      text: entry.text,
      enabled: entry.enabled ?? true,
      accelerator: entry.accelerator,
      action: () => entry.run(),
    };
    return entry.checked === undefined ? base : { ...base, checked: entry.checked };
  });
  const menu = await Menu.new({ items });
  await menu.popup(at ? new LogicalPosition(at.x, at.y) : undefined);
}
