// What Settings holds (APP-DESIGN #17): its sections and rows, in order,
// with the words that find them. Settings search filters these rows, and
// the command palette offers each row (APP-DESIGN #20: "Ctrl+K reaches every
// setting").
import { t, type PlainMessageKey } from "@boltget/ui/messages";
import type { IconName } from "../Icon.svelte";

export type SectionId = "general" | "downloads" | "connections" | "network" | "shortcuts" | "about";

export interface Section {
  id: SectionId;
  label: PlainMessageKey;
  icon: IconName;
}

export const SECTIONS: readonly Section[] = [
  { id: "general", label: "settings.section.general", icon: "settings" },
  { id: "downloads", label: "settings.section.downloads", icon: "folder" },
  { id: "connections", label: "settings.section.connections", icon: "active" },
  { id: "network", label: "settings.section.network", icon: "globe" },
  { id: "shortcuts", label: "settings.section.shortcuts", icon: "keyboard" },
  { id: "about", label: "settings.section.about", icon: "info" },
];

export type RowId =
  | "theme"
  | "autostart"
  | "tray"
  | "notifications"
  | "clipboard"
  | "folder"
  | "categories"
  | "connections"
  | "limit"
  | "proxy"
  | "shortcuts"
  | "about";

export interface Row {
  id: RowId;
  section: SectionId;
  label: PlainMessageKey;
  description?: PlainMessageKey;
  keywords?: PlainMessageKey;
}

export const ROWS: readonly Row[] = [
  {
    id: "theme",
    section: "general",
    label: "settings.theme",
    description: "settings.theme.description",
    keywords: "keywords.theme",
  },
  {
    id: "autostart",
    section: "general",
    label: "settings.autostart",
    description: "settings.autostart.description",
    keywords: "keywords.autostart",
  },
  {
    id: "tray",
    section: "general",
    label: "settings.tray",
    description: "settings.tray.description",
    keywords: "keywords.tray",
  },
  {
    id: "notifications",
    section: "general",
    label: "settings.notifications",
    description: "settings.notifications.description",
    keywords: "keywords.notifications",
  },
  {
    id: "clipboard",
    section: "general",
    label: "settings.clipboard",
    description: "settings.clipboard.description",
    keywords: "keywords.clipboard",
  },
  {
    id: "folder",
    section: "downloads",
    label: "settings.folder",
    description: "settings.folder.description",
    keywords: "keywords.download_folder",
  },
  {
    id: "categories",
    section: "downloads",
    label: "settings.categories",
    description: "settings.categories.description",
    keywords: "keywords.categories",
  },
  {
    id: "connections",
    section: "connections",
    label: "settings.connections",
    description: "settings.connections.description",
    keywords: "keywords.connections",
  },
  {
    id: "limit",
    section: "connections",
    label: "settings.limit",
    description: "settings.limit.description",
    keywords: "keywords.limit",
  },
  {
    id: "proxy",
    section: "network",
    label: "settings.proxy",
    description: "settings.proxy.description",
    keywords: "keywords.proxy",
  },
  {
    id: "shortcuts",
    section: "shortcuts",
    label: "settings.section.shortcuts",
    description: "settings.shortcuts.description",
    keywords: "keywords.shortcuts",
  },
  { id: "about", section: "about", label: "settings.section.about", keywords: "keywords.about" },
];

/** The words a row is found by: its label, description and synonyms. */
export function rowText(row: Row): string {
  return [row.label, row.description, row.keywords]
    .flatMap((key) => (key ? [t(key)] : []))
    .join(" ")
    .toLowerCase();
}

/**
 * The rows a settings search finds: every word of the query appears in the
 * row's label, description or synonyms. An empty query finds none (the
 * sections show instead).
 */
export function searchRows(query: string, rows: readonly Row[] = ROWS): Row[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return [];
  return rows.filter((row) => {
    const text = rowText(row);
    return words.every((w) => text.includes(w));
  });
}

export function sectionOf(id: RowId): SectionId {
  return (ROWS.find((r) => r.id === id) as Row).section;
}
