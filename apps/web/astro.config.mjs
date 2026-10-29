// boltget.app: Astro (static output) with Svelte islands and Starlight docs.
// See docs/WEBSITE.md and docs/TECH-STACK.md (ADR-010).
import { defineConfig } from "astro/config";
import svelte from "@astrojs/svelte";
import starlight from "@astrojs/starlight";
import sitemap from "@astrojs/sitemap";

export default defineConfig({
  site: "https://boltget.app",
  output: "static",
  trailingSlash: "ignore",
  // Marketing pages ship no JavaScript except the hero island (docs/WEBSITE.md).
  prefetch: false,
  integrations: [
    svelte(),
    // Added explicitly (Starlight would add it otherwise) so the design lab
    // pages under /lab/ stay out of the sitemap. They are also noindex.
    sitemap({ filter: (page) => !new URL(page).pathname.startsWith("/lab/") }),
    starlight({
      title: "BoltGet docs",
      description: "How to install and use BoltGet, the download manager built for speed.",
      logo: {
        dark: "../../brand/svg/lockup-horizontal-on-dark.svg",
        light: "../../brand/svg/lockup-horizontal-on-light.svg",
        alt: "BoltGet",
        replacesTitle: true,
      },
      favicon: "/favicon.svg",
      head: [
        { tag: "link", attrs: { rel: "icon", href: "/favicon.ico", sizes: "48x48" } },
        { tag: "link", attrs: { rel: "apple-touch-icon", href: "/apple-touch-icon.png" } },
        { tag: "meta", attrs: { name: "theme-color", content: "#0B0B0F" } },
        { tag: "meta", attrs: { property: "og:image", content: "https://boltget.app/og-image.png" } },
        { tag: "meta", attrs: { name: "twitter:card", content: "summary_large_image" } },
      ],
      social: [
        { icon: "github", label: "GitHub", href: "https://github.com/boltget/boltget" },
      ],
      editLink: {
        baseUrl: "https://github.com/boltget/boltget/edit/main/apps/web/",
      },
      customCss: ["./src/styles/starlight.css"],
      disable404Route: true,
      credits: false,
      sidebar: [
        {
          label: "Getting started",
          items: [
            { label: "Overview", slug: "docs" },
            { label: "Install from source", slug: "docs/getting-started" },
          ],
        },
        {
          label: "CLI",
          items: [
            { label: "Using the CLI", slug: "docs/cli" },
            { label: "Exit codes and JSON", slug: "docs/cli/scripting" },
          ],
        },
      ],
    }),
  ],
});
