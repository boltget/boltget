// The background: a service worker on Chrome and Edge, an event page on
// Firefox (docs/BROWSER-INTEGRATION.md, "Capture flow"). It watches new
// downloads, decides with lib/capture.ts, and hands the ones it captures to
// the app through lib/handoff.ts, which keeps the browser's download unless
// the app accepts it. It also notices the media pages load (lib/media.ts,
// with site access only) for the popup and the toolbar badge, and sends a
// page to the app's media sheet (`media_page`), never on the sites the store
// policy excludes (lib/policy.ts). Every listener is registered synchronously
// at the top, so an event that wakes the worker reaches it.
import { browser, type Browser } from "wxt/browser";
import { defineBackground } from "wxt/utils/define-background";
import { shouldCapture, isFetchable, type CaptureEvent } from "../lib/capture";
import { collectLinks } from "../lib/collect";
import { collectCookies } from "../lib/cookies";
import { handOff } from "../lib/handoff";
import { HeaderLog } from "../lib/headers";
import { t } from "../lib/i18n";
import {
  badgeText,
  classifyResponse,
  hintsToSend,
  readMediaRequest,
  TabMediaStore,
  type MediaSendResult,
  type MediaState,
} from "../lib/media";
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
  sessionArea,
  setMediaBadge,
  setToolbarIcon,
  STORE_POLICY,
} from "../lib/platform";
import { hintAllowed, mediaAllowed } from "../lib/policy";
import { captureSettings, loadSettings } from "../lib/settings";
import { RulesSync } from "../lib/sync";

const MENU = {
  link: "boltget-link",
  selection: "boltget-selection",
  page: "boltget-page",
  sendPage: "boltget-send-page",
  media: "boltget-media",
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
    // The app's rules: asked for at start and before a download when stale.
    const rules = new RulesSync(
      () =>
        client.request({
          type: "hello",
          extension: extensionVersion(),
          browser: detectBrowser(),
        }),
      localStore,
    );

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

    // --- Media the pages load (only with site access) ----------------------

    const tabMedia = new TabMediaStore(sessionArea, (tabId, count) => {
      void setMediaBadge(tabId, badgeText(count));
    });

    /** The page a tab shows: as last seen, else as the browser says. */
    async function tabPage(tabId: number): Promise<string | null> {
      const seen = (await tabMedia.get(tabId)).page;
      if (seen !== null) return seen;
      try {
        return (await browser.tabs.get(tabId)).url ?? null;
      } catch {
        return null;
      }
    }

    browser.webRequest.onHeadersReceived.addListener(
      (details) => {
        if (details.tabId < 0) return;
        const hint = classifyResponse(details);
        if (!hint) return;
        const extra = details as { initiator?: string; documentUrl?: string; originUrl?: string };
        const from = [extra.initiator, extra.documentUrl, extra.originUrl];
        if (!hintAllowed(hint.url, from, STORE_POLICY)) return;
        void (async () => {
          // Nothing is noticed on a page the policy excludes.
          if (!mediaAllowed(await tabPage(details.tabId), STORE_POLICY)) return;
          await tabMedia.add(details.tabId, hint);
          log("media", details.tabId, hint.kind, hint.url);
        })().catch((error: unknown) => log("media failed", error));
      },
      { urls: ["<all_urls>"], types: ["media", "xmlhttprequest", "other"] },
      ["responseHeaders"],
    );

    // A tab that moves to another page starts again.
    browser.webRequest.onBeforeRequest.addListener(
      (details) => {
        if (details.tabId >= 0) void tabMedia.navigated(details.tabId, details.url);
      },
      { urls: ["<all_urls>"], types: ["main_frame"] },
    );
    browser.tabs.onUpdated.addListener((tabId, change, tab) => {
      if (change.url) void tabMedia.navigated(tabId, change.url);
      if (tab.active && (change.url || change.status === "complete")) syncMenus(tab.url);
    });
    browser.tabs.onRemoved.addListener((tabId) => {
      void tabMedia.forget(tabId);
    });
    browser.tabs.onActivated.addListener(({ tabId }) => {
      browser.tabs.get(tabId).then(
        (tab) => syncMenus(tab.url),
        () => {},
      );
    });

    // --- Messages: modifier keys from the content script, the popup --------

    browser.runtime.onMessage.addListener((message, sender, sendResponse) => {
      if (sender.id !== browser.runtime.id) return;
      // The extension's own pages (the popup, also when opened as a tab).
      if (sender.url?.startsWith(browser.runtime.getURL("/"))) {
        const request = readMediaRequest(message);
        if (!request) return;
        const answer =
          request.type === "boltget:media-state"
            ? mediaState(request.tabId)
            : sendFromPopup(request.tabId, request.hint, request.sources);
        answer.then(sendResponse, (error: unknown) => {
          log("popup request failed", error);
          sendResponse(null);
        });
        return true;
      }
      if (!sender.tab || !isLinkClick(message)) return;
      modifiers.record(message.href, modifiersOf(message.alt, message.shift), Date.now());
    });

    /** What the popup shows for a tab. */
    async function mediaState(tabId: number): Promise<MediaState> {
      let url: string | undefined;
      try {
        url = (await browser.tabs.get(tabId)).url;
      } catch {
        return { allowed: false, hints: [] };
      }
      if (!mediaAllowed(url, STORE_POLICY)) return { allowed: false, hints: [] };
      const { hints } = await tabMedia.get(tabId);
      return { allowed: true, hints: hints.filter((h) => hintAllowed(h.url, [], STORE_POLICY)) };
    }

    async function sendFromPopup(
      tabId: number,
      hint: string | null,
      sources: string[],
    ): Promise<MediaSendResult> {
      const tab = await browser.tabs.get(tabId);
      return sendMediaPage(tab, tab.url ?? null, { hint, sources, mediaUrl: null });
    }

    // --- Downloads ---------------------------------------------------------

    async function onDownload(item: Browser.downloads.DownloadItem, filename: string) {
      if (seen.has(item.id) || item.state !== "in_progress") return;
      seen.add(item.id);
      if (seen.size > 1000) seen.clear();

      // A change made in the app applies to this download.
      await rules.fresh(Date.now());
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
      browser.contextMenus.create({
        id: MENU.media,
        title: t("menu.media"),
        contexts: ["video", "audio"],
        documentUrlPatterns: http,
      });
    }

    /**
     * The page and media items are hidden on the sites the store policy
     * excludes, as far as the active tab's address is known (with site
     * access, or once the popup was opened there); a click there still sends
     * nothing.
     */
    function syncMenus(url: string | undefined) {
      const visible = !(isFetchable(url) && !mediaAllowed(url, STORE_POLICY));
      for (const id of [MENU.sendPage, MENU.media]) {
        browser.contextMenus.update(id, { visible }).catch(() => {
          // Not created yet: createMenus runs at start.
        });
      }
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

    /**
     * Sends a page to the app's media sheet (`media_page`): its cookies (and
     * those of the sites its media come from), the browser's User-Agent, the
     * page as Referer, and the hints: `hint` alone, or every one the tab saw
     * with the page's own `sources`. Never for a page the policy excludes,
     * and no hint from an excluded host.
     */
    async function sendMediaPage(
      tab: Browser.tabs.Tab | undefined,
      pageUrl: string | null,
      choice: { hint: string | null; sources: string[]; mediaUrl: string | null },
    ): Promise<MediaSendResult> {
      if (!mediaAllowed(pageUrl, STORE_POLICY) || !pageUrl) return "excluded";
      const seen = tab?.id !== undefined ? (await tabMedia.get(tab.id)).hints : [];
      const allowed = (url: string) => hintAllowed(url, [], STORE_POLICY);
      const hints = hintsToSend(seen, choice.sources.filter(allowed), choice.hint).filter((h) =>
        allowed(h.url),
      );
      const mediaUrl = choice.mediaUrl && allowed(choice.mediaUrl) ? choice.mediaUrl : null;
      // Cookies for the page and for each site its media come from.
      const perSite = new Map<string, string>([[new URL(pageUrl).origin, pageUrl]]);
      for (const url of [...hints.map((h) => h.url), mediaUrl]) {
        if (!url || perSite.size >= 20) continue;
        const origin = new URL(url).origin;
        if (!perSite.has(origin)) perSite.set(origin, url);
      }
      const cookies = await collectCookies(getAllCookies, [...perSite.values()], {
        storeId: tabStore(tab),
        pageUrl,
        firefox: FIREFOX,
      });
      const message = mediaPageMessage({
        pageUrl,
        title: tab?.title,
        incognito: tab?.incognito ?? false,
        cookies,
        headers: [...requestHeaders(), ["referer", pageUrl]],
        hints,
        mediaUrl,
      });
      if (!message) return "failed";
      const result = await client.request({ type: "media_page", ...message });
      log("media page", pageUrl, result);
      if (result.type === "accepted") return "sent";
      // An app from before video (v0.4) refuses hints, or the page.
      if (
        (result.type === "error" && result.reason === "app_too_old") ||
        (result.type === "rejected" && result.reason === "unsupported")
      ) {
        return "app_too_old";
      }
      return "failed";
    }

    async function sendPage(
      info: Browser.contextMenus.OnClickData,
      tab: Browser.tabs.Tab | undefined,
      mediaUrl: string | null,
    ) {
      const pageUrl = info.pageUrl ?? tab?.url ?? null;
      const result = await sendMediaPage(tab, pageUrl, { hint: null, sources: [], mediaUrl });
      if (result === "sent") return;
      if (result === "excluded") {
        // Only worth a word on a page the menu could be on.
        if (isFetchable(pageUrl)) await leaveNotice("notice.media_excluded");
        return;
      }
      await leaveNotice(
        result === "app_too_old" ? "notice.media_unsupported" : "notice.page_failed",
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
            return sendPage(info, tab, null);
          case MENU.media:
            return sendPage(info, tab, isFetchable(info.srcUrl) ? info.srcUrl : null);
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
      log("hello", await rules.now(Date.now()));
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
