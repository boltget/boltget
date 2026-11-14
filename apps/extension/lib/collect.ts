// The link collector for "Download selected links" and "Download all links".
// It is injected with `scripting.executeScript({ func })` only when the user
// picks one of those menu items (activeTab), so it must be self-contained: no
// imports, no closures, its result plain data.

/** A link found on the page. */
export interface CollectedLink {
  url: string;
  text: string;
}

/** The page's links, or those inside the selection when `selectionOnly`. */
export function collectLinks(selectionOnly: boolean): CollectedLink[] {
  const anchors = Array.from(
    document.querySelectorAll<HTMLAnchorElement | HTMLAreaElement>("a[href], area[href]"),
  );
  let chosen = anchors;
  if (selectionOnly) {
    const selection = document.getSelection();
    const ranges: Range[] = [];
    for (let i = 0; selection && i < selection.rangeCount; i++)
      ranges.push(selection.getRangeAt(i));
    chosen = anchors.filter((a) => ranges.some((r) => r.intersectsNode(a)));
  }
  return chosen.map((a) => ({
    url: a.href,
    text: (a.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 300),
  }));
}
