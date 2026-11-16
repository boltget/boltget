// Hover prefetch for every same-site link, through the browser's speculation
// rules (JSON, no script to download): "moderate" prefetches after a short
// hover or on pointer down, so a cross-document view transition finds the
// next page ready. Browsers without speculation rules just navigate.
// Used by layouts/Base.astro and the Starlight head (astro.config.mjs).
export const SPECULATION = JSON.stringify({
  prefetch: [{ where: { href_matches: "/*" }, eagerness: "moderate" }],
});
