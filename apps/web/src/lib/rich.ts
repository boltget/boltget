// Section ledes on Home colour a few key phrases in the section's tint. The
// catalog marks them with square brackets ("Boltget [reads the server
// first]"), so the copy stays one whole sentence in en.json. This escapes the
// text and turns each bracketed phrase into a highlight span.
const ESCAPES: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" };

/** HTML for `text` with `[phrase]` rendered as `<span class="hl">phrase</span>`. */
export function highlight(text: string): string {
  const safe = text.replace(/[&<>"]/g, (c) => ESCAPES[c] ?? c);
  return safe.replace(/\[([^\]]+)\]/g, '<span class="hl">$1</span>');
}
