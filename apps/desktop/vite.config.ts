/// <reference types="vitest/config" />
import { readFileSync } from "node:fs";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig, type Plugin } from "vite";

// The theme colours resolved from the brand tokens (packages/ui generates them).
const colors = JSON.parse(
  readFileSync(new URL("../../packages/ui/src/tokens/theme.json", import.meta.url), "utf8"),
) as Record<"light" | "dark", Record<string, string>>;

/**
 * Inlines the window background into index.html, so the very first paint is
 * the theme's colour even before the stylesheet arrives (APP-DESIGN #30).
 * The window itself is hidden until the app's first frame; this is the second
 * line of defence against a white or black flash.
 */
function prePaintBackground(): Plugin {
  const css =
    `html{background:${colors.light.content};color-scheme:light dark}` +
    `@media (prefers-color-scheme: dark){html:not([data-theme=light]){background:${colors.dark.content}}}` +
    `html[data-theme=dark]{background:${colors.dark.content}}`;
  return {
    name: "boltget:pre-paint-background",
    transformIndexHtml: () => [{ tag: "style", children: css, injectTo: "head-prepend" }],
  };
}

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [svelte(), prePaintBackground()],
  // Tauri expects a fixed port and shows its own errors.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    // WebView2 is evergreen Chromium.
    target: "chrome120",
    minify: !process.env.TAURI_ENV_DEBUG,
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
  test: {
    include: ["src/**/*.test.ts"],
  },
});
