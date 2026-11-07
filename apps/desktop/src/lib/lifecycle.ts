// The moment the window becomes visible. Motion that must be seen, such as
// the empty state's mark playing once, waits for it.
let resolveShown: () => void = () => {};

export const windowShown = new Promise<void>((resolve) => {
  resolveShown = resolve;
});

export function markWindowShown(): void {
  performance.mark("boltget:shown");
  resolveShown();
}

let listMarked = false;

/**
 * Called when the downloads are first known: marks the first frame that can
 * show them as `boltget:list` (a `performance` mark), and the window's first
 * frame as `boltget:shown`. Start-up measurements read both through the
 * DevTools protocol (docs/APP-PERFORMANCE.md); nothing else uses them.
 */
export function markListKnown(): void {
  if (listMarked || typeof requestAnimationFrame !== "function") return;
  listMarked = true;
  void windowShown.then(() => requestAnimationFrame(() => performance.mark("boltget:list")));
}
