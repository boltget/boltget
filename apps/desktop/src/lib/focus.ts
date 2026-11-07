// Keeps Tab inside a modal dialog. A native modal <dialog> makes the page
// behind it inert, but in WebView2 Tab after the last control leaves the page
// for the host window, so one Tab stop lands nowhere and a screen reader
// announces the document. This wraps Tab from the last control to the first
// and Shift+Tab from the first to the last (docs/ACCESSIBILITY.md).

const FOCUSABLE =
  "a[href], button, input, select, textarea, [tabindex]:not([tabindex='-1']), [contenteditable='true']";

/** The controls Tab can reach in `root`, in order. */
export function tabbable(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
    (el) =>
      !el.matches(":disabled") &&
      el.tabIndex >= 0 &&
      !el.closest("[inert], [hidden]") &&
      el.getClientRects().length > 0,
  );
}

/** Svelte action for a modal `<dialog>`: Tab wraps around inside it. */
export function trapTab(dialog: HTMLElement) {
  function onkeydown(event: KeyboardEvent) {
    if (event.key !== "Tab" || event.ctrlKey || event.altKey || event.metaKey) return;
    const items = tabbable(dialog);
    const first = items[0];
    const last = items[items.length - 1];
    if (!first || !last) return;
    const active = document.activeElement;
    if (event.shiftKey && (active === first || !dialog.contains(active))) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && (active === last || !dialog.contains(active))) {
      event.preventDefault();
      first.focus();
    }
  }
  dialog.addEventListener("keydown", onkeydown);
  return { destroy: () => dialog.removeEventListener("keydown", onkeydown) };
}
