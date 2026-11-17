// The thin layer between the pure logic and the browser: which build this is,
// which browser it runs in, and the browser APIs wrapped in the shapes lib/
// takes. Everything here runs only inside the extension.
import type { Browser as ProtocolBrowser } from "@boltget/native-messaging";
import { browser } from "wxt/browser";
import type { BrowserCookie, CookieQuery } from "./cookies";
import type { DownloadControl, DownloadState } from "./handoff";
import {
  readScan,
  scanPage,
  type MediaSendResult,
  type MediaState,
  type PageScan,
  type SessionArea,
} from "./media";
import { hostName, NativeClient, type Flavor } from "./native";
import type { Store } from "./settings";

/** Release builds talk to the installed app, development builds to the dev host. */
export const FLAVOR: Flavor = import.meta.env.MODE === "production" ? "release" : "development";

/** The Firefox build (event page, `browser.*` with Firefox's extras). */
export const FIREFOX = import.meta.env.FIREFOX;

/** Set by wxt.config.ts at build time. */
declare const __BOLTGET_STORE_POLICY__: boolean;

/**
 * The store policy (lib/policy.ts): no media on YouTube's sites. On in every
 * build unless built with BOLTGET_STORE_POLICY=off, for a browser no store
 * sees (docs/BROWSER-INTEGRATION.md, "Store policy constraints").
 */
export const STORE_POLICY: boolean = __BOLTGET_STORE_POLICY__;

/** `storage.session` for lib/media.ts's TabMediaStore. */
export const sessionArea: SessionArea = {
  get: (key) => browser.storage.session.get(key),
  set: (items) => browser.storage.session.set(items),
  remove: (key) => browser.storage.session.remove(key),
};

/** Shows a tab's media count on the toolbar button ("" hides it). */
export async function setMediaBadge(tabId: number, text: string): Promise<void> {
  try {
    await browser.action.setBadgeText({ tabId, text });
    if (text) await browser.action.setBadgeBackgroundColor({ tabId, color: "#0A0A0B" });
  } catch {
    // The tab closed meanwhile.
  }
}

/** The browser, for `hello` (the Chrome build also runs in Edge and others). */
export function detectBrowser(): ProtocolBrowser {
  if (FIREFOX) return "firefox";
  const data = (navigator as { userAgentData?: { brands?: { brand: string }[] } }).userAgentData;
  const brands = data?.brands?.map((b) => b.brand) ?? [];
  const ua = navigator.userAgent;
  if (brands.includes("Microsoft Edge") || /\bEdg\//.test(ua)) return "edge";
  // Headless builds say "HeadlessChrome/".
  if (brands.some((b) => b.includes("Chrom")) || /Chrome\//.test(ua)) return "chrome";
  return "other";
}

/** The extension's version, for `hello`. */
export function extensionVersion(): string {
  return browser.runtime.getManifest().version;
}

/** The stub host the end-to-end tests register (e2e/stub-host.mjs). */
export const E2E_HOST_NAME = "app.boltget.native_host_e2e";

/** A native messaging client for this build's host. */
export function nativeClient(): NativeClient {
  return new NativeClient(
    (host, message) => browser.runtime.sendNativeMessage(host, message),
    import.meta.env.MODE === "e2e" ? E2E_HOST_NAME : hostName(FLAVOR),
  );
}

/** `storage.local` as a {@link Store}. */
export const localStore: Store = {
  get: (keys) => browser.storage.local.get(keys),
  set: (items) => browser.storage.local.set(items),
};

/** `cookies.getAll`, with the query fields only some browsers know. */
export function getAllCookies(query: CookieQuery): Promise<BrowserCookie[]> {
  return browser.cookies.getAll(query as Parameters<typeof browser.cookies.getAll>[0]);
}

/** The `downloads` API as the handoff uses it. */
export const downloadControl: DownloadControl = {
  pause: (id) => browser.downloads.pause(id),
  resume: (id) => browser.downloads.resume(id),
  cancel: (id) => browser.downloads.cancel(id),
  erase: async (id) => {
    await browser.downloads.erase({ id });
  },
  async get(id): Promise<DownloadState | null> {
    const [item] = await browser.downloads.search({ id });
    if (!item) return null;
    return {
      state: item.state as DownloadState["state"],
      paused: item.paused,
      canResume: item.canResume,
      error: item.error,
      url: item.url,
    };
  },
  restart: async (url) => {
    await browser.downloads.download({ url, conflictAction: "uniquify" });
  },
};

/** Whether the user granted access to all sites (cookies, headers, the content script). */
export function hasSiteAccess(): Promise<boolean> {
  return browser.permissions.contains({ origins: ["<all_urls>"] });
}

/** Asks for access to all sites; must run in a click handler. */
export function requestSiteAccess(): Promise<boolean> {
  return browser.permissions.request({ origins: ["<all_urls>"] });
}

/** Gives access to all sites back. */
export function removeSiteAccess(): Promise<boolean> {
  return browser.permissions.remove({ origins: ["<all_urls>"] });
}

/** Notices the background leaves for the popup (a menu action that needs a word). */
export type NoticeKey =
  | "notice.media_unsupported"
  | "notice.media_excluded"
  | "notice.link_kept"
  | "notice.links_failed"
  | "notice.page_failed"
  | "notice.no_links";

/** Leaves a notice: a badge on the toolbar button, and the popup if the browser lets us open it. */
export async function leaveNotice(key: NoticeKey): Promise<void> {
  await browser.storage.session.set({ notice: key });
  await browser.action.setBadgeText({ text: "!" });
  await browser.action.setBadgeBackgroundColor({ color: "#0A0A0B" });
  try {
    await browser.action.openPopup();
  } catch {
    // Older browsers, or no focused window: the badge stays until the popup opens.
  }
}

/** Takes the notice for the popup, clearing the badge. */
export async function takeNotice(): Promise<NoticeKey | null> {
  const { notice } = await browser.storage.session.get("notice");
  await browser.storage.session.remove("notice");
  await browser.action.setBadgeText({ text: "" });
  return typeof notice === "string" ? (notice as NoticeKey) : null;
}

/** The toolbar icon for a dark or a light toolbar (Chromium; Firefox uses theme_icons). */
export async function setToolbarIcon(dark: boolean): Promise<void> {
  if (FIREFOX) return;
  const variant = dark ? "toolbar-on-dark" : "toolbar";
  await browser.action.setIcon({
    path: Object.fromEntries([16, 24, 32].map((s) => [s, `/icons/${variant}-${s}.png`])),
  });
}

/**
 * An extension page (popup, options) reports the browser's colour scheme:
 * the toolbar icon follows it, now and at the next start.
 */
export async function reportColorScheme(dark: boolean): Promise<void> {
  if (FIREFOX) return;
  const { toolbarDark } = await browser.storage.local.get("toolbarDark");
  if (toolbarDark === dark) return;
  await browser.storage.local.set({ toolbarDark: dark });
  await setToolbarIcon(dark);
}

/**
 * The tab the popup is about: the active tab of its window. The end-to-end
 * build also takes `?tab=<id>`, since a test opens the popup as a page of its
 * own.
 */
export async function popupTab(): Promise<number | null> {
  if (import.meta.env.MODE === "e2e") {
    const id = Number(new URLSearchParams(location.search).get("tab") ?? "");
    if (Number.isInteger(id) && id > 0) return id;
  }
  const [tab] = await browser.tabs.query({ active: true, currentWindow: true });
  return tab?.id ?? null;
}

/** What tab `tabId` has seen of media, from the background (null when it cannot say). */
export async function tabMediaState(tabId: number): Promise<MediaState | null> {
  try {
    return ((await browser.runtime.sendMessage({ type: "boltget:media-state", tabId })) ??
      null) as MediaState | null;
  } catch {
    return null;
  }
}

/** Asks the background to send tab `tabId`'s page to the app's media sheet. */
export async function sendTabMedia(
  tabId: number,
  hint: string | null,
  sources: string[],
): Promise<MediaSendResult> {
  try {
    const result: unknown = await browser.runtime.sendMessage({
      type: "boltget:media-send",
      tabId,
      hint,
      sources,
    });
    return typeof result === "string" ? (result as MediaSendResult) : "failed";
  } catch {
    return "failed";
  }
}

/**
 * Looks at the page itself once (scanPage, injected through the popup's
 * activeTab grant: no script stays in the page). Nothing when the browser
 * does not allow it there.
 */
export async function scanTab(tabId: number): Promise<PageScan> {
  try {
    const [first] = await browser.scripting.executeScript({ target: { tabId }, func: scanPage });
    return readScan(first?.result);
  } catch {
    return readScan(null);
  }
}
