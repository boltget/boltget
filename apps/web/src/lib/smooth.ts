// Gentle smooth scrolling on the marketing pages (docs/WEBSITE.md, Motion).
// Only the mouse wheel and trackpad are smoothed, lightly (SMOOTHING below);
// touch, the keyboard, the scrollbar and find-in-page stay native. Mouse
// clicks on in-page links ("See how it works") glide to their target below
// the sticky header. Not loaded on the docs (Starlight), nor under reduced
// motion or the ?at= review freeze.
//
// This replaces Lenis (November 2026, for the JavaScript budget): the same
// frame-rate independent damping Lenis uses with `lerp`, in about 1 KB
// instead of 5. The page still scrolls natively underneath, so the scroll
// reveals, the header's floating state and the demos' visibility checks are
// unchanged.

/** How much the wheel is smoothed: the share of the remaining distance
 * covered each frame at 60 fps. 1 is native; lower is smoother. Kept light on
 * purpose, so it feels like native scrolling, just less jerky. Tune here only. */
const SMOOTHING = 0.15;

export function smoothScroll(): void {
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  if (document.documentElement.classList.contains("frozen")) return;

  const header = document.querySelector<HTMLElement>("[data-header]");
  const offset = (header?.offsetHeight ?? 72) + 8;
  const root = document.documentElement;
  const limit = () => root.scrollHeight - innerHeight;
  let target = scrollY;
  let current = scrollY;
  let frame = 0;
  let last = 0;

  const step = (now: number) => {
    // Something else moved the page (a key, the scrollbar): let it win.
    if (Math.abs(scrollY - Math.round(current)) > 2) {
      frame = 0;
      return;
    }
    const dt = Math.min(0.1, (now - last) / 1000);
    last = now;
    current += (target - current) * (1 - Math.exp(-SMOOTHING * 60 * dt));
    if (Math.abs(target - current) < 0.5) current = target;
    scrollTo({ top: current, behavior: "instant" });
    frame = current === target ? 0 : requestAnimationFrame(step);
  };

  const glide = (to: number) => {
    if (!frame) {
      current = scrollY;
      last = performance.now();
      frame = requestAnimationFrame(step);
    }
    target = Math.max(0, Math.min(limit(), to));
  };

  // True when an element under the pointer can still scroll that way itself.
  const nested = (e: WheelEvent) => {
    for (let el = e.target instanceof Element ? e.target : null; el &&el !== root && el !== document.body; el = el.parentElement) {
      if (el.scrollHeight <= el.clientHeight || !/auto|scroll/.test(getComputedStyle(el).overflowY)) continue;
      if (e.deltaY < 0 ? el.scrollTop > 0 : el.scrollTop + el.clientHeight < el.scrollHeight - 1) return true;
    }
    return false;
  };

  addEventListener(
    "wheel",
    (e) => {
      // Pinch and ctrl+wheel zoom, sideways scrolling and inner scrollers stay native.
      if (e.ctrlKey || e.defaultPrevented || Math.abs(e.deltaX) > Math.abs(e.deltaY) || nested(e)) return;
      e.preventDefault();
      const unit = e.deltaMode === 1 ? 100 / 6 : e.deltaMode === 2 ? innerHeight : 1;
      glide((frame ? target : scrollY) + e.deltaY * unit);
    },
    { passive: false },
  );

  // Mouse clicks on in-page links glide; keyboard activation (detail 0)
  // stays native, so focus moves as usual (the skip link).
  document.addEventListener("click", (e) => {
    if (e.defaultPrevented || e.button || e.detail === 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    const a = (e.target as Element).closest?.<HTMLAnchorElement>("a[href*='#']");
    if (!a || a.origin !== location.origin || a.pathname !== location.pathname || a.search !== location.search) return;
    const el = a.hash.length > 1 ? document.getElementById(decodeURIComponent(a.hash.slice(1))) : null;
    if (!el && a.hash.length > 1) return;
    e.preventDefault();
    history.pushState(null, "", a.hash || location.pathname);
    glide(el ? el.getBoundingClientRect().top + scrollY - offset : 0);
  });
}
