// The background: a service worker on Chrome and Edge, an event page on
// Firefox (docs/BROWSER-INTEGRATION.md, "Capture flow"). It watches new
// downloads, decides with lib/capture.ts, and hands the ones it captures to
// the app through lib/handoff.ts, which keeps the browser's download unless
// the app accepts it. Every listener is registered synchronously at the top,
// so an event that wakes the worker reaches it.
import { browser, type Browser } from "wxt/browser";
import { defineBackground } from "wxt/utils/define-background";
import { shouldCapture, isFetchable, type CaptureEvent } from "../lib/capture";
import { collectLinks } from "../lib/collect";
import { collectCookies } from "../lib/cookies";
import { handOff } from "../lib/handoff";
import { HeaderLog } from "../lib/headers";
import { t } from "../lib/i18n";
import { addLinksMessage, captureMessage, mediaPageMessage } from "../lib/message";
import { isLinkClick, ModifierLog, modifiersOf } from "../lib/modifiers";
import {
  detectBrowser,
  downloadControl,
  extensionVersion,
  FIREFOX,
  FLAVOR,
  getAllCookies,
  hasSiteAccess,
  leaveNotice,
  localStore,
  nativeClient,
  setToolbarIcon,
} from "../lib/platform";
import { onAnySite } from "../lib/rules";
import { adoptAppRules, captureSettings, loadSettings } from "../lib/settings";

/** Sites where the store builds offer no page or media actions (store policy). */
const MEDIA_EXCLUDED = ["youtube.com", "youtu.be", "youtube-nocookie.com"];

const MENU = {
  link: "boltget-link",
  selection: "boltget-selection",
  page: "boltget-page",
  sendPage: "boltget-send-page",
} as const;

const CONTENT_SCRIPT_ID = "boltget-modifiers";

function log(...args: unknown[]) {
  if (FLAVOR === "development") console.info("[boltget]", ...args);
}

export default defineBackground({
  // Firefox: an event page, not a persistent background.
  persistent: false,
  main() {
    const headers = new HeaderLog();
    const modifiers = new ModifierLog();
    const client = nativeClient();
    const seen = new Set<number>();

    // --- Observed request headers (only with site access) -----------------

    const extraInfo = ["requestHeaders"] as `${Browser.webRequest.OnSendHeadersOptions}`[];
    if (!FIREFOX) extraInfo.push("extraHeaders");
    browser.webRequest.onSendHeaders.addListener(
      (details) => {
        headers.record(details.url, details.method, details.requestHeaders, Date.now());
      },
      { urls: ["<all_urls>"], types: ["main_frame", "sub_frame", "other"] },
      extraInfo,
    );

    // --- Modifier keys from the content script -----------------------------

    browser.runtime.onMessage.addListener((message, sender) => {
      if (sender.id !== browser.runtime.id || !sender.tab || !isLinkClick(message)) return;
      modifiers.record(message.href, modifiersOf(message.alt, message.shift), Date.now());
    });

    // --- Downloads ---------------------------------------------------------

    async function onDownload(item: Browser.downloads.DownloadItem, filename: string) {
      if (seen.has(item.id) || item.state !== "in_progress") return;
      seen.add(item.id);
      if (seen.size > 1000) seen.clear();

      const now = Date.now();
      const settings = await loadSettings(localStore);
      const observed = headers.take([item.finalUrl, item.url], now);
      const keys = settings.modifierKeys ? modifiers.take([item.url, item.finalUrl], now) : "none";
      const event: CaptureEvent = {
        url: item.url,
        finalUrl: item.finalUrl,
        filename,
        mime: item.mime,
        size: item.totalBytes > 0 ? item.totalBytes : item.fileSize > 0 ? item.fileSize : null,
        referrer: item.referrer,
        incognito: item.incognito,
        method: observed?.method ?? null,
        byExtension: !!item.byExtensionId,
        modifiers: keys,
      };
      const decision = shouldCapture(event, captureSettings(settings, now));
      log("download", item.id, item.url, decision);
      if (!decision.capture) return;

      const storeId = FIREFOX
        ? (item as { cookieStoreId?: string }).cookieStoreId
        : item.incognito
          ? "1"
          : "0";
      const handoff = await handOff(
        item.id,
        async () => {
          const cookies = await collectCookies(getAllCookies, [item.url, item.finalUrl], {
            storeId,
            pageUrl: item.referrer,
            firefox: FIREFOX,
          });
          const message = captureMessage({
            url: item.url,
            finalUrl: item.finalUrl,
            filename,
            size: event.size,
            mime: item.mime,
            referrer: item.referrer,
            pageUrl: null,
            incognito: item.incognito,
            forced: decision.forced,
            cookies,
            headers: observed?.headers ?? [],
          });
          return client.request({ type: "capture", ...message });
        },
        downloadControl,
      );
      log("handoff", item.id, handoff.outcome, handoff.result);
    }

    if (!FIREFOX && browser.downloads.onDeterminingFilename) {
      // Chromium: called once the response's headers are known, with the
      // browser's file name. Accept the browser's name at once (we never
      // change it), then decide.
      browser.downloads.onDeterminingFilename.addListener((item, suggest) => {
        suggest();
        void onDownload(item, item.filename);
      });
    } else {
      // Firefox: the item has its full target path by the time it is created.
      browser.downloads.onCreated.addListener((item) => {
        void onDownload(item, item.filename);
      });
    }

    // --- Context menus -----------------------------------------------------

    async function createMenus() {
      await browser.contextMenus.removeAll();
      const http = ["http://*/*", "https://*/*"];
      browser.contextMenus.create({
        id: MENU.link,
        title: t("menu.link"),
        contexts: ["link"],
        targetUrlPatterns: http,
      });
      browser.contextMenus.create({
        id: MENU.selection,
        title: t("menu.selection"),
        contexts: ["selection"],
        documentUrlPatterns: http,
      });
      browser.contextMenus.create({
        id: MENU.page,
        title: t("menu.page"),
        contexts: ["page"],
        documentUrlPatterns: http,
      });
      browser.contextMenus.create({
        id: MENU.sendPage,
        title: t("menu.send_page"),
        contexts: ["page"],
        documentUrlPatterns: http,
      });
    }

    function requestHeaders(): [string, string][] {
      return [["user-agent", navigator.userAgent]];
    }

    function tabStore(tab: Browser.tabs.Tab | undefined): string | undefined {
      if (FIREFOX) return (tab as { cookieStoreId?: string } | undefined)?.cookieStoreId;
      return tab?.incognito ? "1" : "0";
    }

    async function downloadLink(info: Browser.contextMenus.OnClickData, tab?: Browser.tabs.Tab) {
      const url = info.linkUrl;
      if (!isFetchable(url)) return;
      const pageUrl = info.pageUrl ?? tab?.url ?? null;
      const incognito = tab?.incognito ?? false;
      const cookies = await collectCookies(getAllCookies, [url], {
        storeId: tabStore(tab),
        pageUrl,
        firefox: FIREFOX,
      });
      const message = captureMessage({
        url,
        referrer: pageUrl,
        pageUrl,
        incognito,
        forced: true,
        cookies,
        headers: pageUrl ? [...requestHeaders(), ["referer", pageUrl]] : requestHeaders(),
      });
      const result = await client.request({ type: "capture", ...message });
      log("menu link", url, result);
      if (result.type === "accepted") return;
      // The browser keeps it: there is no browser download yet, so start one.
      // It is ours (byExtensionId), so capture leaves it alone.
      await browser.downloads.download({ url });
      await leaveNotice("notice.link_kept");
    }

    async function downloadLinks(
      info: Browser.contextMenus.OnClickData,
      tab: Browser.tabs.Tab | undefined,
      selectionOnly: boolean,
    ) {
      if (!tab?.id) return;
      const frameIds = info.frameId !== undefined ? [info.frameId] : undefined;
      const results = await browser.scripting.executeScript({
        target: frameIds ? { tabId: tab.id, frameIds } : { tabId: tab.id },
        func: collectLinks,
        args: [selectionOnly],
      });
      const links = results.flatMap((r) => (Array.isArray(r.result) ? r.result : []));
      const pageUrl = info.frameUrl ?? info.pageUrl ?? tab.url ?? null;
      // Cookies for each site the links are on (the first link of each).
      const perSite = new Map<string, string>();
      for (const link of links) {
        try {
          const origin = new URL(link.url).origin;
          if (!perSite.has(origin) && perSite.size < 20) perSite.set(origin, link.url);
        } catch {
          // Not a URL: cleanLinks drops it.
        }
      }
      const cookies = await collectCookies(getAllCookies, [...perSite.values()], {
        storeId: tabStore(tab),
        pageUrl,
        firefox: FIREFOX,
      });
      const message = addLinksMessage(
        links,
        pageUrl,
        tab.incognito,
        cookies,
        pageUrl ? [...requestHeaders(), ["referer", pageUrl]] : requestHeaders(),
      );
      if (!message) {
        await leaveNotice("notice.no_links");
        return;
      }
      const result = await client.request({ type: "add_links", ...message });
      log("menu links", message.links.length, result);
      if (result.type !== "accepted") await leaveNotice("notice.links_failed");
    }

    async function sendPage(info: Browser.contextMenus.OnClickData, tab?: Browser.tabs.Tab) {
      const pageUrl = info.pageUrl ?? tab?.url ?? null;
      if (onAnySite([pageUrl], MEDIA_EXCLUDED)) {
        await leaveNotice("notice.media_excluded");
        return;
      }
      const cookies = pageUrl
        ? await collectCookies(getAllCookies, [pageUrl], {
            storeId: tabStore(tab),
            pageUrl,
            firefox: FIREFOX,
          })
        : [];
      const message = mediaPageMessage(pageUrl, tab?.title, tab?.incognito ?? false, cookies);
      if (!message) return;
      const result = await client.request({ type: "media_page", ...message });
      log("menu page", pageUrl, result);
      if (result.type === "accepted") return;
      await leaveNotice(
        result.type === "rejected" && result.reason === "unsupported"
          ? "notice.media_unsupported"
          : "notice.page_failed",
      );
    }

    browser.contextMenus.onClicked.addListener((info, tab) => {
      const run = async () => {
        switch (info.menuItemId) {
          case MENU.link:
            return downloadLink(info, tab);
          case MENU.selection:
            return downloadLinks(info, tab, true);
          case MENU.page:
            return downloadLinks(info, tab, false);
          case MENU.sendPage:
            return sendPage(info, tab);
        }
      };
      run().catch((error: unknown) => log("menu failed", error));
    });

    // --- The modifier-key content script (only with site access) ----------

    async function syncContentScript() {
      const settings = await loadSettings(localStore);
      const want = settings.modifierKeys && (await hasSiteAccess());
      const registered = await browser.scripting.getRegisteredContentScripts({
        ids: [CONTENT_SCRIPT_ID],
      });
      if (want && registered.length === 0) {
        await browser.scripting.registerContentScripts([
          {
            id: CONTENT_SCRIPT_ID,
            js: ["modifiers.js"],
            matches: ["http://*/*", "https://*/*"],
            allFrames: true,
            runAt: "document_start",
          },
        ]);
      } else if (!want && registered.length > 0) {
        await browser.scripting.unregisterContentScripts({ ids: [CONTENT_SCRIPT_ID] });
      }
    }

    function sync() {
      syncContentScript().catch((error: unknown) => log("content script", error));
    }

    browser.permissions.onAdded.addListener(sync);
    browser.permissions.onRemoved.addListener(sync);
    browser.storage.onChanged.addListener((changes, area) => {
      if (area === "local" && "modifierKeys" in changes) sync();
    });

    // --- Start-up ----------------------------------------------------------

    /** Asks the host who is there; takes the app's rules when it is running. */
    async function hello() {
      const result = await client.request({
        type: "hello",
        extension: extensionVersion(),
        browser: detectBrowser(),
      });
      if (result.type === "hello" && result.rules) await adoptAppRules(localStore, result.rules);
      log("hello", result);
    }

    async function start() {
      await createMenus();
      const { toolbarDark } = await browser.storage.local.get("toolbarDark");
      if (toolbarDark === true) await setToolbarIcon(true);
      await syncContentScript();
      await hello();
    }

    browser.runtime.onInstalled.addListener((details) => {
      if (details.reason === "install") {
        void browser.tabs.create({ url: browser.runtime.getURL("/options.html#welcome") });
      }
      start().catch((error: unknown) => log("start", error));
    });
    browser.runtime.onStartup.addListener(() => {
      start().catch((error: unknown) => log("start", error));
    });
  },
});
