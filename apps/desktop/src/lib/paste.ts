// Links arrive from outside the sheet: Ctrl+V in the window, a dropped link,
// Ctrl+N and the "Add download" button. All of them open the add sheet with
// the links filled in (APP-DESIGN #6); nothing starts without it.
import { app } from "./app.svelte";
import { linksIn } from "./add/links";

/** How long the clipboard may take before the sheet opens without it. */
const CLIPBOARD_WAIT_MS = 300;

/** Opens the sheet with the links in `text`; false when there is none. */
export function openAddFromText(text: string): boolean {
  const links = linksIn(text);
  if (links.length === 0) return false;
  app.openAdd(links.join("\n"));
  return true;
}

/** Ctrl+N and the "Add download" button: the sheet, with the clipboard's links. */
export async function openAddFromClipboard(): Promise<void> {
  let text = "";
  try {
    // A clipboard that does not answer (no focus, no permission) must not
    // keep the sheet closed: after a moment it opens empty.
    text = await Promise.race([
      navigator.clipboard.readText(),
      new Promise<string>((resolve) => setTimeout(() => resolve(""), CLIPBOARD_WAIT_MS)),
    ]);
  } catch {
    // No clipboard access: the sheet opens empty.
  }
  app.openAdd(linksIn(text).join("\n"));
}
