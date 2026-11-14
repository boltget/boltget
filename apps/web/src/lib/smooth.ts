// Gentle smooth scrolling on the marketing pages (docs/WEBSITE.md, Motion),
// with Lenis (MIT). Only the mouse wheel and trackpad are smoothed, lightly
// (SMOOTHING below); touch scrolling stays native. In-page links ("See how it
// works") glide to their target below the sticky header. Not loaded on the
// docs (Starlight), nor under reduced motion or the ?at= review freeze.
// The page still scrolls natively underneath, so the scroll reveals, the
// header's floating state and the demos' visibility checks are unchanged.
import Lenis from "lenis";

/** How much the wheel is smoothed: the share of the remaining distance
 * covered each frame. 1 is native; lower is smoother. Kept light on purpose,
 * so it feels like native scrolling, just less jerky. Tune here only. */
const SMOOTHING = 0.15;

export function smoothScroll(): void {
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  if (document.documentElement.classList.contains("frozen")) return;
  const header = document.querySelector<HTMLElement>("[data-header]");
  const offset = -((header?.offsetHeight ?? 72) + 8);
  new Lenis({
    lerp: SMOOTHING,
    wheelMultiplier: 1,
    smoothWheel: true,
    syncTouch: false,
    autoRaf: true,
    anchors: { offset },
  });
}
