// Categories by file type (APP-DESIGN #2), derived from the file name's
// extension: the engine has no categories of its own yet.
import type { IconName } from "../Icon.svelte";

export const CATEGORIES = ["video", "music", "archives", "programs", "documents", "other"] as const;
export type Category = (typeof CATEGORIES)[number];

// Space-separated, lowercase, without the dot.
const EXTENSIONS: Record<Exclude<Category, "other">, string> = {
  video: "mp4 mkv avi mov webm wmv flv m4v mpg mpeg m2ts 3gp",
  music: "mp3 flac wav aac ogg oga m4a opus wma aiff alac mka",
  archives: "zip rar 7z tar gz tgz bz2 xz zst lz lzma cab iso img",
  programs: "exe msi msix msixbundle appx appxbundle dmg pkg deb rpm apk appimage flatpak snap",
  documents: "pdf doc docx xls xlsx ppt pptx odt ods odp rtf txt md csv epub mobi",
};

const BY_EXTENSION = new Map<string, Category>(
  Object.entries(EXTENSIONS).flatMap(([category, list]) =>
    list.split(" ").map((ext) => [ext, category as Category] as const),
  ),
);

/** The category of a file, from its name (`ubuntu.iso` is an archive). */
export function categoryOf(name: string): Category {
  const dot = name.lastIndexOf(".");
  if (dot <= 0 || dot === name.length - 1) return "other";
  return BY_EXTENSION.get(name.slice(dot + 1).toLowerCase()) ?? "other";
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
