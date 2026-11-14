// Smooth <details> where CSS cannot do it yet (docs/WEBSITE.md, Motion).
// Browsers with interpolate-size and ::details-content animate disclosures in
// global.css; in the others this eases the content's height and opacity with
// the Web Animations API. Under reduced motion nothing is animated.

const EASE = "cubic-bezier(0.22, 1, 0.36, 1)";

export function smoothDetails(): void {
  if (CSS.supports("interpolate-size: allow-keywords")) return;
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
  for (const details of document.querySelectorAll<HTMLDetailsElement>("details")) {
    const summary = details.querySelector(":scope > summary");
    const body = summary?.nextElementSibling as HTMLElement | null;
    if (!summary || !body) continue;
    let running: Animation | null = null;
    summary.addEventListener("click", (e) => {
      e.preventDefault();
      running?.cancel();
      const opening = !details.open;
      if (opening) details.open = true;
      const full = body.scrollHeight;
      body.style.overflow = "clip";
      running = body.animate(
        opening
          ? [{ height: "0px", opacity: 0 }, { height: `${full}px`, opacity: 1 }]
          : [{ height: `${full}px`, opacity: 1 }, { height: "0px", opacity: 0 }],
        { duration: 340, easing: EASE },
      );
      running.onfinish = () => {
        if (!opening) details.open = false;
        body.style.overflow = "";
        running = null;
      };
    });
  }
}
