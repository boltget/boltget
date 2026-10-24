# Browser integration

The BoltGet extension catches downloads in the browser and hands them to the BoltGet app together with everything the app needs to fetch the same file (cookies, referrer, headers). It also detects media on pages and offers to send the page to the app. It does no downloading itself and contains no site-specific extraction logic.

- Code: `apps/extension` (planned)
- Framework: [WXT](https://wxt.dev), Manifest V3, TypeScript, Svelte 5 for the popup and options UI
- Targets: Chrome, Microsoft Edge, Firefox. Other Chromium browsers (Brave, Vivaldi, Opera) are expected to work with the Chrome build but are not tested targets for v0.3.
- Transport to the app: [native messaging](#native-messaging), never a localhost server

Related: [ARCHITECTURE.md](ARCHITECTURE.md#process-model) (native host process), [ENGINE.md](ENGINE.md#request-context) (what the request context is used for), [VIDEO.md](VIDEO.md) (what happens to media).

## Design goals

1. **Never lose a download.** If the app is not installed, not running and cannot be started, or rejects the download, the browser keeps it. The worst case is "the browser downloaded it like normal".
2. **Capture what the user expects, nothing else.** Big files and archives yes; a 40 KB PDF the user wants to read in the browser no.
3. **Minimal permissions, honestly explained.** Every permission is justified in the store listing and in onboarding.
4. **Store-compliant.** One codebase, builds that satisfy Chrome Web Store, Edge Add-ons and Firefox Add-ons policies.

## Extension structure (WXT)

```
apps/extension/
├── wxt.config.ts            # manifest generation per browser
├── entrypoints/
│   ├── background.ts        # service worker (Chrome/Edge) / event page (Firefox)
│   ├── content.ts           # modifier-key tracking, link collection (minimal, injected on demand)
│   ├── popup/               # Svelte: status, capture toggle, media list for the current tab
│   └── options/             # Svelte: capture rules, site lists, connection status
├── lib/
│   ├── capture.ts           # capture decision logic (pure, unit-tested)
│   ├── native.ts            # native messaging client, reconnect, protocol version
│   ├── headers.ts           # request header observation and allowlist
│   └── media.ts             # media detection
└── public/icons/            # generated from brand/icon.svg
```

The capture decision is a pure function `shouldCapture(event, rules) → Decision` so it can be tested exhaustively without a browser.

## Permissions

| Permission | Why |
|---|---|
| `downloads` | Observe new downloads, pause/cancel/erase the browser's copy after the app accepts it |
| `nativeMessaging` | Talk to the BoltGet app |
| `cookies` | Read cookies for the download URL so the app can make the same authenticated request |
| `webRequest` | Observe (not block, on Chrome) request headers such as `Referer` and `User-Agent`, and response `Content-Type` for media detection |
| `contextMenus` | "Download with BoltGet", "Download all links", "Send page to BoltGet" |
| `storage` | Capture rules and site lists |
| `scripting` | Inject the link collector only when the user invokes "Download all links" |
| host permissions | Needed by `cookies` and `webRequest` for the sites involved. Requested as **optional** `<all_urls>` during onboarding with an explanation, so the user explicitly grants them. Without them, capture still works but without cookies/headers (many downloads will then fail in the app, and the extension says so). |

Not requested: `tabs` beyond what `activeTab` gives, `history`, `bookmarks`, `<all_urls>` content scripts at install time, `webRequestBlocking` on Chrome (not available to normal MV3 extensions).

## Capture flow

```mermaid
sequenceDiagram
  participant B as Browser
  participant E as Extension (background)
  participant H as Native host
  participant A as BoltGet app

  B->>E: downloads.onCreated / onDeterminingFilename (item)
  E->>E: shouldCapture(item, rules, modifier state)?
  alt do not capture
    E-->>B: (nothing; browser continues)
  else capture
    E->>B: downloads.pause(item.id)
    E->>E: collect cookies (cookies.getAll), observed request headers, tab URL
    E->>H: capture {url, finalUrl, filename, size, mime, referrer, cookies, headers}
    H->>A: forward over named pipe (launch app if needed)
    A-->>H: accepted / rejected
    H-->>E: reply
    alt accepted
      E->>B: downloads.cancel(item.id), downloads.erase({id})
    else rejected, error, or timeout
      E->>B: downloads.resume(item.id)
    end
  end
```

### Chromium (Chrome, Edge)

Manifest V3 extensions cannot block network requests with `webRequest`, so capture works through the `downloads` API:

1. `chrome.downloads.onCreated` gives the `DownloadItem` (URL, final URL, referrer, MIME, size if known, incognito flag). `onDeterminingFilename` additionally gives the browser's proposed filename once headers are known.
2. The extension **pauses** the browser download immediately, then asks the app.
3. Only after the app replies "accepted" does the extension cancel and erase the browser's download. If the app is unreachable within a timeout (design target: 3 s including app launch), the download is resumed in the browser.

Caveat: by the time the extension sees the download, the browser has already sent the request. For **one-time links** (a token valid for a single request), the app's request may then be refused. In that case the app replies `rejected: link_consumed` after its probe fails, and the extension resumes the browser's paused download, which still holds the original response. The user gets the file either way; it just isn't accelerated.

### Firefox

Firefox's Manifest V3 implementation still supports blocking `webRequest` (`webRequestBlocking`). The Firefox build can therefore decide in `webRequest.onHeadersReceived` (where `Content-Disposition`, `Content-Type` and `Content-Length` are visible) and cancel the request *before* the browser starts a download, which is cleaner and avoids the double-request problem for most cases. The `downloads` API path is kept as a fallback for downloads that start without a matching observed request (for example `blob:` URLs, which the extension ignores, or downloads initiated by other extensions).

Whether to use blocking `webRequest` on Firefox in v0.3 or keep a single code path is decided during implementation; the capture decision function is shared either way.

### What is never captured

- `blob:` and `data:` URLs (the bytes only exist inside the page; the app cannot fetch them).
- Downloads from incognito/private windows, unless the user has enabled the extension in private windows **and** turned on "capture in private windows" in options. Private downloads never appear in the app's history after completion (opt-in setting).
- `file:` URLs.
- Requests with a `POST` body that the extension cannot see (form-submission downloads). These stay in the browser.
- Downloads started by the BoltGet app itself (none in v1, but guarded).

## Capture rules

Defaults (all editable in options, synced from the app's settings on `hello` so both agree):

| Rule | Default |
|---|---|
| File types | Archives (`zip`, `rar`, `7z`, `tar`, `gz`, `bz2`, `xz`, `zst`), disc images (`iso`, `img`), installers and packages (`exe`, `msi`, `msix`, `appx`, `dmg`, `pkg`, `deb`, `rpm`, `apk`), video (`mp4`, `mkv`, `webm`, `mov`, `avi`, `m4v`), audio (`mp3`, `flac`, `wav`, `m4a`, `ogg`, `opus`), large data (`bin`, `dat`, `torrent` is **not** included). Documents (`pdf`, `docx`, ...) are **not** captured by default. |
| MIME types | `application/octet-stream` is captured only when the extension matches the list above or the size passes the threshold |
| Size threshold | Capture only if size ≥ 1 MiB (design target) or size unknown and type matches. Small files are faster in the browser than any handoff. |
| Site lists | Never-capture list (for example internal tools, banking). Always-capture list. Both match on host suffix. |
| Global toggle | Popup switch; also "Pause capture for 10 minutes" |

Matching uses, in order: the filename from `Content-Disposition`/browser suggestion, then the URL path extension, then MIME type.

### Modifier-key bypass

- **Bypass (let the browser download):** hold **Alt** while clicking the link (default; configurable).
- **Force capture:** hold **Alt + Shift** while clicking, regardless of rules.

Implementation: a small content script records `altKey`/`shiftKey` on `mousedown`/`click` for links and posts `{tabId, href, modifiers, t}` to the background, which keeps it for a short window (design target: 2 s). When a download from that tab with a matching URL appears, the recorded modifiers apply.

Notes:

- In Chromium, Alt+click on a link already means "download link" by default, so Alt fits naturally: the user is asking the browser to download it.
- The content script is injected only on pages where host permission was granted. Without it, modifier bypass is unavailable and the popup toggle is the bypass.

## Context menus

| Context | Item | Message |
|---|---|---|
| Link | Download with BoltGet | `capture` (forced) for the link URL, with cookies for it and the page as referrer |
| Selection | Download selected links with BoltGet | Content script collects `<a href>` within the selection → `add_links` |
| Page | Download all links with BoltGet... | Content script collects all links → `add_links`; the app opens a picker with filters by type |
| Video / audio element | Download media with BoltGet | `media_page` with the page URL and element source hint |
| Page | Send page to BoltGet | `media_page` with the page URL |

Media-related items follow the [store policy constraints](#store-policy-constraints).

## Cookies, headers and referrer forwarding

For each captured download the extension sends:

- **Cookies:** `cookies.getAll({ url })` for the download URL and, if different, the final URL, as structured records (name, value, domain, path, secure, httpOnly, sameSite, expirationDate). For partitioned cookies (CHIPS), the extension also queries with the top-level site's `partitionKey` where the browser supports it. The app builds a cookie jar from these records (see [ENGINE.md, Authentication and cookies](ENGINE.md#authentication-and-cookies)).
- **Request headers** observed in `webRequest.onSendHeaders` for the matching request (Chrome needs the `extraHeaders` option to expose `Referer` and `Cookie`), filtered through the allowlist: `User-Agent`, `Referer`, `Accept`, `Accept-Language`, `Origin`, `Authorization`. Observed headers are kept in a short-lived map keyed by request ID and URL (design target: 30 s, size-capped) and discarded after use.
- **Referrer:** `DownloadItem.referrer`, falling back to the tab URL.
- **Filename hint and size hint** from the browser.

Nothing is sent for downloads that are not captured. Nothing is stored by the extension beyond the short-lived header map.

## Native messaging

### Protocol

Defined in [ARCHITECTURE.md, IPC and events](ARCHITECTURE.md#2-extension-to-app-native-messaging-through-the-host). Framing (browser-defined): each message is a 32-bit length in native byte order followed by UTF-8 JSON. Chromium limits messages from the host to the browser to 1 MB; messages to the host can be much larger, but BoltGet's host rejects anything over 1 MiB anyway.

The background script uses `runtime.connectNative` while it needs a session (for example while the popup is open) and `runtime.sendNativeMessage` for one-off captures, because MV3 service workers are terminated when idle and must not depend on a long-lived port.

### Host manifest

Host name: `app.boltget.native_host` (Chromium requires lowercase letters, digits, underscores and dots).

Chromium manifest (`app.boltget.native_host.chrome.json`, in the install directory):

```json
{
  "name": "app.boltget.native_host",
  "description": "BoltGet native messaging host",
  "path": "boltget-native-host.exe",
  "type": "stdio",
  "allowed_origins": [
    "chrome-extension://<chrome-web-store-id>/",
    "chrome-extension://<edge-add-ons-id>/"
  ]
}
```

On Windows, `path` may be relative to the manifest file's directory.

Firefox manifest (`app.boltget.native_host.firefox.json`):

```json
{
  "name": "app.boltget.native_host",
  "description": "BoltGet native messaging host",
  "path": "C:\\Users\\<user>\\AppData\\Local\\Programs\\BoltGet\\boltget-native-host.exe",
  "type": "stdio",
  "allowed_extensions": ["extension@boltget.app"]
}
```

The Firefox extension ID is fixed in the manifest (`browser_specific_settings.gecko.id = "extension@boltget.app"`). Chromium IDs are assigned by each store; the Edge Add-ons ID differs from the Chrome Web Store ID, so both are listed. Development builds use a pinned development key and register a separate host name (`app.boltget.native_host_dev`) so a development extension cannot talk to a production install.

### Native messaging host registration on Windows

The installer writes one registry key per browser. The key's **default value** is the full path to the manifest file.

Per-user install (default NSIS installer, no admin rights):

| Browser | Registry key |
|---|---|
| Chrome (and Chromium browsers that read Chrome's location) | `HKEY_CURRENT_USER\Software\Google\Chrome\NativeMessagingHosts\app.boltget.native_host` |
| Microsoft Edge | `HKEY_CURRENT_USER\Software\Microsoft\Edge\NativeMessagingHosts\app.boltget.native_host` |
| Firefox | `HKEY_CURRENT_USER\Software\Mozilla\NativeMessagingHosts\app.boltget.native_host` |

Per-machine install (MSI): the same paths under `HKEY_LOCAL_MACHINE` instead. Chromium checks the 32-bit registry view first and then the 64-bit view; writing to the native view of a 64-bit installer is sufficient, but the installer verifies registration by reading back through both views.

Other Chromium browsers (Brave, Vivaldi, Opera) may read Chrome's key or their own vendor-specific key; which ones do is to be verified during v0.3 and documented here.

The uninstaller removes all keys and manifest files. The app's settings page has "Repair browser integration", which rewrites keys and manifests (useful after moving the install directory) and shows per-browser status.

macOS and Linux (post-1.0) use manifest files in each browser's `NativeMessagingHosts` directory instead of registry keys.

### Connection status

The popup shows one of: Connected (app version), App not running (it will start when needed), App not installed (link to download), App too old (update), Extension too old (update). The options page has a "Test connection" button that sends `hello`.

## Media detection panel

Media detection only **detects and hands off**. All extraction, format selection and downloading happens in the app through yt-dlp ([VIDEO.md](VIDEO.md)).

- Detection sources:
  - `webRequest.onHeadersReceived` responses with `Content-Type` `video/*`, `audio/*`, `application/vnd.apple.mpegurl` / `application/x-mpegurl` (HLS), `application/dash+xml` (DASH).
  - URL patterns `.m3u8`, `.mpd`, `.mp4`, `.webm` when the content type is generic.
  - `<video>` / `<audio>` elements with a direct `src` (via the context menu only, no persistent content script).
- Noise filtering: HLS/DASH media segments (`.ts`, `.m4s`, byte-range fragments) are grouped under their manifest and not listed individually; responses smaller than a threshold (design target: 512 KiB) with no manifest are ignored; ad and tracking hosts are not specially handled, they simply rarely meet the thresholds.
- UI: the toolbar icon shows a small badge with the number of detected media items on the current tab. The popup lists them (type, resolution if derivable from the manifest URL, size if known). Choosing one sends `media_page` with the **page URL** and the detected media URL as a hint. The app runs yt-dlp on the page URL; the hint is used only when yt-dlp has no extractor for the site (generic direct-media fallback).
- No floating "download this video" button is injected into pages in v0.4. It is intrusive and the most policy-sensitive design. Reconsider later as an opt-in.

## Store policy constraints

- **Chrome Web Store:** Google's policies have long disallowed extensions that enable downloading from YouTube, and reviews are strict about it. The Chrome Web Store build therefore:
  - shows **no** media detection, media context-menu items or "send page" action on YouTube domains (`youtube.com`, `youtu.be`, `youtube-nocookie.com`, `music.youtube.com`), and
  - says nothing about YouTube in its listing.
  The app itself accepts any URL the user pastes; that is the app's business, not the extension's.
- **Edge Add-ons:** treated the same as Chrome (same build).
- **Firefox Add-ons:** has different rules, but for one consistent product the Firefox build applies the same YouTube exclusion by default. This can be revisited.
- **No remote code** (MV3 rule, and a BoltGet security rule): all extension logic is bundled; capture rules come from the app as data, never code.
- **Single purpose:** the listing describes one purpose, "send downloads to the BoltGet download manager", with media detection as part of that purpose.
- **Privacy disclosures:** the extension collects no data for BoltGet; cookies and headers go only to the local app over native messaging. The listing's data-use section states exactly this.
- Policies change. Before each store submission, the release checklist ([RELEASE.md](RELEASE.md)) includes re-reading the current program policies.

## Testing

- Unit tests for `shouldCapture`, header filtering, media grouping (Vitest).
- End-to-end tests with Playwright driving Chromium with the unpacked extension, against the engine's local test server, with a stub native host that records messages.
- A manual capture matrix before each release: direct links, redirects, signed URLs, cookie-protected downloads, `Content-Disposition` names, downloads started by JavaScript, downloads in iframes, Alt-bypass, private windows.
