// Paste-to-add, the stand-in for the add sheet (step 4 of v0.2 builds the
// real one, APP-DESIGN #6): Ctrl+V in the window, a dropped link or the
// "Add download" button starts the http(s) links found in the text at once.
import { t } from "@boltget/ui/messages";
import { app } from "./app.svelte";
import { downloads } from "./engine";
import { displayName } from "./format";

/** The http and https links in a pasted or dropped text, without repeats. */
export function linksIn(text: string): string[] {
  const found = new Set<string>();
  for (const word of text.split(/\s+/)) {
    const candidate = word.replace(/^[<("']+|[>)"',.;]+$/g, "");
    try {
      const url = new URL(candidate);
      if (url.protocol === "http:" || url.protocol === "https:") found.add(url.href);
    } catch {
      // Not a link.
    }
  }
  return [...found];
}

/** Starts every link in `text`; false when there was none. */
export function addFromText(text: string): boolean {
  const links = linksIn(text);
  for (const url of links) {
    void downloads.add({ url }).then((result) => {
      if (result.status === "ok") {
        const name = displayName(result.data.fileName, result.data.url);
        app.toast(t("toast.added", { name }));
      }
    });
  }
  return links.length > 0;
}

/** The "Add download" button: takes a link from the clipboard. */
export async function addFromClipboard(): Promise<void> {
  let text = "";
  try {
    text = await navigator.clipboard.readText();
  } catch {
    // No clipboard access: fall through to the hint.
  }
  if (!addFromText(text)) app.toast(t("add.hint"));
}
