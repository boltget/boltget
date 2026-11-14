// WXT configuration: one codebase, a Chrome/Edge and a Firefox build, each in
// a release and a development flavor (docs/BROWSER-INTEGRATION.md).
//
// - `wxt build` (mode `production`) is the release flavor: store IDs, host
//   `app.boltget.native_host`.
// - `wxt` / `wxt build --mode development` is the development flavor: the
//   pinned development key (so an unpacked Chromium build gets the ID in
//   packages/native-messaging/extensions.json), host
//   `app.boltget.native_host_dev`, and Firefox `extension-dev@boltget.app`.
// - `wxt build --mode e2e` is the development flavor for the end-to-end tests
//   (e2e/): it talks to `app.boltget.native_host_e2e`, a stub the tests
//   register, and holds site access from the start, because a test cannot
//   answer the browser's permission prompt.
import { defineConfig, type ConfigEnv } from "wxt";
import extensions from "../../packages/native-messaging/extensions.json" with { type: "json" };

const ICON_SIZES = [16, 32, 48, 128] as const;
// The toolbar button has its own icons, the arrow drawn solid per size
// (scripts/icons.mjs), because the brand's gapped pixel arrow breaks up there.
const TOOLBAR_SIZES = [16, 24, 32] as const;

function toolbarIcons(variant: "toolbar" | "toolbar-on-dark"): Record<number, string> {
  return Object.fromEntries(TOOLBAR_SIZES.map((size) => [size, `icons/${variant}-${size}.png`]));
}

function icons(variant: "icon" | "icon-on-dark"): Record<number, string> {
  return Object.fromEntries(ICON_SIZES.map((size) => [size, `icons/${variant}-${size}.png`]));
}

function manifest({ mode, browser }: ConfigEnv) {
  const development = mode !== "production";
  const e2e = mode === "e2e";
  const firefox = browser === "firefox";
  const ids = development ? extensions.development : extensions.release;

  return {
    // Store listing text lives in _locales so the stores can localise it later.
    name: development ? "__MSG_extension_name_dev__" : "__MSG_extension_name__",
    short_name: "Boltget",
    description: "__MSG_extension_description__",
    default_locale: "en",
    homepage_url: "https://boltget.app",
    icons: icons("icon"),
    action: {
      default_icon: toolbarIcons("toolbar"),
      default_title: "__MSG_extension_action_title__",
      // Firefox swaps the toolbar icon with its theme. Chromium has no
      // manifest equivalent; the background swaps it from what the popup
      // and options pages report (lib/icon.ts).
      ...(firefox
        ? {
            theme_icons: TOOLBAR_SIZES.map((size) => ({
              light: `icons/toolbar-${size}.png`,
              dark: `icons/toolbar-on-dark-${size}.png`,
              size,
            })),
          }
        : {}),
    },
    permissions: [
      "downloads",
      "nativeMessaging",
      "cookies",
      "webRequest",
      "contextMenus",
      "storage",
      "scripting",
      "activeTab",
    ],
    // Granted by the user during onboarding (options page), never at install.
    ...(e2e ? { host_permissions: ["<all_urls>"] } : { optional_host_permissions: ["<all_urls>"] }),
    ...(firefox
      ? {
          browser_specific_settings: {
            gecko: {
              id: ids.firefox[0],
              strict_min_version: "128.0",
              // Cookies and headers go only to the local Boltget app.
              data_collection_permissions: { required: ["none"] },
            },
          },
        }
      : {
          minimum_chrome_version: "120",
          // The development build's ID is derived from this public key.
          ...(development && "chromiumKey" in ids ? { key: ids.chromiumKey } : {}),
        }),
  };
}

export default defineConfig({
  srcDir: ".",
  // Firefox too (WXT builds MV2 for Firefox by default).
  manifestVersion: 3,
  imports: false,
  modules: ["@wxt-dev/module-svelte"],
  manifest,
  zip: {
    artifactTemplate: "boltget-extension-{{version}}-{{browser}}{{modeSuffix}}.zip",
    sourcesTemplate: "boltget-extension-{{version}}-sources{{modeSuffix}}.zip",
    // The store listing's images and their scripts are not part of the extension.
    excludeSources: ["store/**"],
  },
  vite: () => ({
    // packages/ui's fonts live in brand/ at the repository root.
    server: { fs: { allow: ["../.."] } },
  }),
});
