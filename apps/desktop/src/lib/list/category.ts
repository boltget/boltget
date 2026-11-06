// Categories by file type (APP-DESIGN #2), derived from the file name's
// extension. The extensions come from Settings (`AppSettings.categories`);
// until they arrive, and in tests, the built-in lists below apply. The
// backend has the same lists (`src-tauri/src/settings.rs`, checked by a
// test there).
import type { PlainMessageKey } from "@boltget/ui/messages";
import type { Category as CategorySetting } from "../bindings";
import type { IconName } from "../Icon.svelte";
import { categoryTable } from "./categoryTable.svelte";

export const CATEGORIES = ["video", "music", "archives", "programs", "documents", "other"] as const;
export type Category = (typeof CATEGORIES)[number];

/** Each category's name in the catalog. */
export const CATEGORY_LABELS: Record<Category, PlainMessageKey> = {
  video: "category.video",
  music: "category.music",
  archives: "category.archives",
  programs: "category.programs",
  documents: "category.documents",
  other: "category.other",
};

// Space-separated, lowercase, without the dot.
export const DEFAULT_EXTENSIONS: Record<Exclude<Category, "other">, string> = {
  video: "mp4 mkv avi mov webm wmv flv m4v mpg mpeg m2ts 3gp",
  music: "mp3 flac wav aac ogg oga m4a opus wma aiff alac mka",
  archives: "zip rar 7z tar gz tgz bz2 xz zst lz lzma cab iso img",
  programs: "exe msi msix msixbundle appx appxbundle dmg pkg deb rpm apk appimage flatpak snap",
  documents: "pdf doc docx xls xlsx ppt pptx odt ods odp rtf txt md csv epub mobi",
};

/** The built-in categories, as Settings holds them. */
export const DEFAULT_CATEGORIES: CategorySetting[] = CATEGORIES.map((id) => ({
  id,
  extensions: id === "other" ? [] : DEFAULT_EXTENSIONS[id].split(" "),
  folder: null,
}));

/** Extension to category, for a list of categories (the first one wins). */
export function tableOf(categories: readonly CategorySetting[]): ReadonlyMap<string, Category> {
  const table = new Map<string, Category>();
  for (const c of categories) {
    if (c.id === "other") continue;
    for (const ext of c.extensions) if (!table.has(ext)) table.set(ext, c.id);
  }
  return table;
}

/** The lowercase extension of a file name, without the dot. */
export function extensionOf(name: string): string | null {
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) return null;
  return name.slice(dot + 1).toLowerCase();
}

/**
 * The category of a file, from its name (`ubuntu.iso` is an archive), with
 * the categories from Settings unless others are given.
 */
export function categoryOf(
  name: string,
  table: ReadonlyMap<string, Category> = categoryTable(),
): Category {
  const ext = extensionOf(name);
  return (ext && table.get(ext)) || "other";
}

const ICONS: Record<Category, IconName> = {
  video: "video",
  music: "music",
  archives: "archives",
  programs: "programs",
  documents: "documents",
  other: "file",
};

/** The type icon of a file, from its name. */
export function categoryIcon(name: string): IconName {
  return ICONS[categoryOf(name)];
}
