# Browser integration

The Boltget extension catches downloads in the browser and hands them to the Boltget app together with everything the app needs to fetch the same file (cookies, referrer, headers). It also detects media on pages and offers to send the page to the app. It does no downloading itself and contains no site-specific extraction logic.

- Code: `apps/extension` (planned); the native host is `crates/native-host`, its protocol `crates/ipc/src/browser.rs`
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
| `nativeMessaging` | Talk to the Boltget app |
| `cookies` | Read cookies for the download URL so the app can make the same authenticated request |
| `webRequest` | Observe (not block, on Chrome) request headers such as `Referer` and `User-Agent`, and response `Content-Type` for media detection |
| `contextMenus` | "Download with Boltget", "Download all links", "Send page to Boltget" |
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
  participant A as Boltget app

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

Caveat: by the time the extension sees the download, the browser has already sent the request. For **one-time links** (a token valid for a single request), the app's request may then be refused. In that case the app replies `rejected` with the reason `link_refused` after its probe fails, and the extension resumes the browser's paused download, which still holds the original response. The user gets the file either way; it just isn't accelerated.

> **Implemented (v0.3 step 1, the app's side; `apps/desktop/src-tauri/src/browser.rs`).** Before it answers a `capture`, the app makes its own first request for the link with the browser's cookies and headers (the engine's probe), within 4 s. A refusal (401, 403, 404, 410, 429 and the like: `link_refused`), any other failure, or no answer in time (`failed`) is a `rejected` reply, and the browser keeps its download: losing a download is worse than not accelerating it. Otherwise the download is added to the main queue in its category's folder and the reply is `accepted` with its id. This costs one extra request per capture (the engine probes again when the download starts); folding the two together is left for later.
>
> **"Ask before adding browser downloads"** (Settings, Browser; off by default, which keeps "zero mandatory decisions on add") opens the add sheet with the link instead, and the reply is `accepted` with `confirm: true` at once: the browser's copy is dropped as the sheet opens, as IDM does with its dialog, because the extension cannot wait for the user. The request context waits in the app's memory, by link, for 30 minutes (at most 5000 links): the sheet's preview and its Download button use it, so the cookies and headers are the browser's. Cancelling the sheet drops the download. `add_links` ("Download all links", a selection's links) always opens the add sheet with the links, which is the picker for now.
>
> Browser cookies are not saved with a download (no secret is stored before DPAPI encryption arrives, [ENGINE.md](ENGINE.md#request-context)): a captured download that is resumed after Boltget restarts runs without them, and asks for a new link if the server wants them. Private-window captures are treated like any other for now (the "never in history" setting is still to do).

### Firefox

Firefox's Manifest V3 implementation still supports blocking `webRequest` (`webRequestBlocking`). The Firefox build can therefore decide in `webRequest.onHeadersReceived` (where `Content-Disposition`, `Content-Type` and `Content-Length` are visible) and cancel the request *before* the browser starts a download, which is cleaner and avoids the double-request problem for most cases. The `downloads` API path is kept as a fallback for downloads that start without a matching observed request (for example `blob:` URLs, which the extension ignores, or downloads initiated by other extensions).

Whether to use blocking `webRequest` on Firefox in v0.3 or keep a single code path is decided during implementation; the capture decision function is shared either way.

### What is never captured

- `blob:` and `data:` URLs (the bytes only exist inside the page; the app cannot fetch them).
- Downloads from incognito/private windows, unless the user has enabled the extension in private windows **and** turned on "capture in private windows" in options. Private downloads never appear in the app's history after completion (opt-in setting).
- `file:` URLs.
- Requests with a `POST` body that the extension cannot see (form-submission downloads). These stay in the browser.
- Downloads started by the Boltget app itself (none in v1, but guarded).

## Capture rules

Defaults (all editable in options, synced from the app's settings on `hello` so both agree):

> **Implemented (v0.3 step 1):** the app answers `hello` with these defaults as `rules` (`enabled`, `min_size` 1 MiB, `extensions`, `never_sites`, `always_sites`, `private_windows`; `CaptureRules` in `crates/ipc/src/browser.rs`). Editing them in the app, so the extension's options and the app agree, is still to do; until then the extension keeps its own edits.

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
| Link | Download with Boltget | `capture` (forced) for the link URL, with cookies for it and the page as referrer |
| Selection | Download selected links with Boltget | Content script collects `<a href>` within the selection → `add_links` |
| Page | Download all links with Boltget... | Content script collects all links → `add_links`; the app opens a picker with filters by type |
| Video / audio element | Download media with Boltget | `media_page` with the page URL and element source hint |
| Page | Send page to Boltget | `media_page` with the page URL |

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

Defined in [ARCHITECTURE.md, IPC and events](ARCHITECTURE.md#2-extension-to-app-native-messaging-through-the-host). Framing (browser-defined): each message is a 32-bit length in native byte order followed by UTF-8 JSON. Chromium limits messages from the host to the browser to 1 MB and sends a host at most 64 MiB; Boltget's host rejects anything over 1 MiB.

> **Implemented (v0.3 step 1), protocol version 1.** The reference is `crates/ipc/src/browser.rs`; its TypeScript types are generated into `packages/native-messaging/src/protocol.ts` (package `@boltget/native-messaging`, regenerated with `pnpm --filter @boltget/native-messaging generate`; a test of `crates/native-host` fails while it is stale), which the extension imports instead of writing message shapes by hand.
>
> - **Envelope.** Every message is a JSON object with `v` (the protocol version, an integer), `type` and an optional `id` (an integer up to 2^32-1) that the reply carries back. Every reply has `v`, `type` and the same `id`.
> - **Messages** (extension to host):
>
>   | `type` | Fields | Reply | Starts the app |
>   |---|---|---|---|
>   | `hello` | `extension` (its version), `browser` (`chrome`, `edge`, `firefox`, `other`) | `hello {app, host, oldest, current, rules}`; `app` and `rules` are `null` when the app is not running | no |
>   | `capture` | `url`; optional `final_url`, `filename`, `size`, `mime`, `referrer`, `page_url`; `incognito`, `forced` (default false); `cookies`, `headers` (default empty) | `accepted`, `rejected`, `unavailable` or `error` | yes |
>   | `add_links` | `links` (`[{url, text?}]`, 1 to 5000), optional `page_url` (also the referrer), `incognito`, `cookies`, `headers` | `accepted {confirm: true}` | yes |
>   | `media_page` | `page_url`, optional `media_url`, `title`, `incognito`, `cookies`, `headers` | `rejected {reason: "unsupported"}` until v0.4 | yes |
>   | `status` | none | `status {active, queued, speed}` | no |
>
>   A cookie is `{name, value, domain, path = "/", host_only, secure, http_only, same_site?, expires?}` as `cookies.getAll` reports it (`same_site`: `no_restriction`, `lax`, `strict`, `unspecified`; `expires` in seconds since 1970, absent for a session cookie). Headers are `[name, value]` pairs from the allowlist below, each name at most once. The app's request goes to `final_url` when there is one, with the browser's `Referer` header, else `referrer`, as the referrer.
> - **Replies.** Only `accepted {download, confirm}` means the app took the download (`download`: its id when it was added at once; `confirm`: the add sheet asks the user). Every other reply means the browser keeps it: `rejected {reason, message}` (`link_refused`, `unsupported`, `failed`), `unavailable {reason, message}` (`not_running` for `status`, `not_installed` when the host finds no app to start, `timeout`, `busy` when a standalone `boltget` owns the downloads, `mismatch` when the host and the app are from different releases) and `error {reason, message}` (`invalid`, `too_large`, `extension_too_old`, `app_too_old`). `message` is an English diagnostic for logs, never UI text.
> - **Strict and bounded.** An unknown `type` or field, or any value out of its limits, refuses the whole message with `error: invalid`: URLs (`url`, `final_url`, `referrer`, `page_url`, link URLs, `media_url`) must be `http`/`https` with a host and at most 32 KiB; file names, link texts and titles at most 1 KiB, a file name without `/`, `\` or control characters; MIME types and the extension's version at most 255 printable characters; at most 300 cookies and 64 KiB of cookie names and values, without `;` or control characters, with a valid domain and a path starting with `/`; header values at most 8 KiB, without CR or LF. A message over 1 MiB is skipped unread and answered with `error: too_large`, so the session stays in step; a frame over 64 MiB, or a stream cut inside a frame, ends the host. The version is checked before the shape, so a newer extension is told `app_too_old` rather than `invalid`. The host checks all of this before relaying, and the app checks it again. The host's reader is fuzzed (`fuzz/fuzz_targets/native_message.rs`, nightly; its seed corpus runs in `cargo test`).
> - **Versions.** A request gaining a field changes the version; a reply gaining one does not (the extension ignores what it does not know). The host and app serve `oldest` to `current` (1 to 1 today).
> - **Timing.** With the app running, a reply takes milliseconds plus the app's first request for a captured link (at most 4 s). When the host has to start the app it waits up to 8 s for it, then up to 6 s for the reply: **the extension's timeout must be at least 15 s**, longer than the 3 s design target above, or a late `accepted` would leave the file downloaded twice. A typical cold start answers in about 2 s.
> - **One connection per download.** A message that may add a download (`capture`, `add_links`, `media_page`) always opens a new pipe connection, so a reply lost on a stale connection can never make the app add it twice; `hello` and `status` reuse the open one.
> - **Starting the app.** The host starts `Boltget.exe --background` from its own folder (a development build's `boltget-desktop.exe` too; `BOLTGET_NATIVE_HOST_APP` names another), detached, with `CREATE_BREAKAWAY_FROM_JOB` so a browser's job object (Firefox runs hosts in one) does not end it with the host, falling back to starting it inside the job when the job forbids that. It reaches the app on the pipe of `BOLTGET_DATA_DIR` or the default data directory, like the CLI.
> - **Not in version 1:** `refresh_link`. Browser-assisted link refresh ([ENGINE.md](ENGINE.md#link-refresh)) needs no message of its own: the app recognises the re-captured download (same name and size) when a `capture` arrives. That matching is not built yet.

The background script uses `runtime.connectNative` while it needs a session (for example while the popup is open) and `runtime.sendNativeMessage` for one-off captures, because MV3 service workers are terminated when idle and must not depend on a long-lived port.

### Host manifest

Host name: `app.boltget.native_host` (Chromium requires lowercase letters, digits, underscores and dots).

Chromium manifest (`app.boltget.native_host.chrome.json`, in the install directory):

```json
{
  "name": "app.boltget.native_host",
  "description": "Boltget native messaging host",
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
  "description": "Boltget native messaging host",
  "path": "C:\\Users\\<user>\\AppData\\Local\\Programs\\Boltget\\boltget-native-host.exe",
  "type": "stdio",
  "allowed_extensions": ["extension@boltget.app"]
}
```

The Firefox extension ID is fixed in the manifest (`browser_specific_settings.gecko.id = "extension@boltget.app"`). Chromium IDs are assigned by each store; the Edge Add-ons ID differs from the Chrome Web Store ID, so both are listed. Development builds use a pinned development key and register a separate host name (`app.boltget.native_host_dev`) so a development extension cannot talk to a production install.

> **Implemented (v0.3 step 1).** The extension IDs live in one file, `packages/native-messaging/extensions.json`, read by the host's registration (compiled in) and later by the extension's build:
>
> - `release`: host `app.boltget.native_host`, Chromium IDs `aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa` (Chrome Web Store) and `bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb` (Edge Add-ons), which are **placeholders** to replace with the store-assigned IDs before the first store release (they are well-formed, so browsers accept the manifest, and belong to no extension), and Firefox `extension@boltget.app`.
> - `development`: host `app.boltget.native_host_dev`, Chromium ID `fckefagfbiinebmgfpgjiielmmmaggbd`, pinned by the public key in `chromiumKey` (the development extension's `manifest.json` carries it as `key`; Chromium derives the ID from it, and a test checks that they match; an unpacked extension needs no private key, and none is kept), and Firefox `extension-dev@boltget.app`.
>
> Debug builds of the host register the development flavor, release builds the release one. The manifests are not shipped as files: the registration writes them next to the host (`<host name>.chrome.json`, `<host name>.firefox.json`) with the host's absolute path, so writing them again repairs a moved install.

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

> **Implemented (v0.3 step 1, per-user only; `crates/native-host/src/registration.rs`).** `boltget-native-host --register` writes both manifests next to itself and the three HKCU keys (Chrome and Edge share the Chromium manifest), `--unregister` removes them, `--status` prints each browser's state. The NSIS installer ships the host next to `Boltget.exe` and runs `--register` after installing (updates included) and `--unregister` before uninstalling, then deletes the keys and manifests itself in case the host is gone ([RELEASE.md](RELEASE.md#installer)). Settings, Browser, lists Chrome, Edge and Firefox as Ready, Not registered, or Needs repair (the key points to another manifest, the manifest to another or a missing host, or it names other extensions than this release's), with a Repair button that runs the same registration for the host next to the app. The registry is written through `winreg` (safe Rust). `HKCU\Software` is shared by the 32- and 64-bit views, so one write serves both. Not done: the MSI's HKLM registration, and checking which other Chromium browsers read Chrome's key.

macOS and Linux (post-1.0) use manifest files in each browser's `NativeMessagingHosts` directory instead of registry keys.

### Connection status

The popup shows one of: Connected (app version), App not running (it will start when needed), App not installed (link to download), App too old (update), Extension too old (update). The options page has a "Test connection" button that sends `hello`.

> **Mapping (v0.3 step 1):** a `hello` reply with `app` set is Connected; with `app: null`, App not running (the host answered, so Boltget is installed); no host at all (`runtime.lastError`, "Specified native messaging host not found") is App not installed; `error: app_too_old` is App too old; `error: extension_too_old` is Extension too old; `unavailable: mismatch` asks to reinstall Boltget.

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
- **No remote code** (MV3 rule, and a Boltget security rule): all extension logic is bundled; capture rules come from the app as data, never code.
- **Single purpose:** the listing describes one purpose, "send downloads to the Boltget download manager", with media detection as part of that purpose.
- **Privacy disclosures:** the extension collects no data for Boltget; cookies and headers go only to the local app over native messaging. The listing's data-use section states exactly this.
- Policies change. Before each store submission, the release checklist ([RELEASE.md](RELEASE.md)) includes re-reading the current program policies.

## Testing

- Unit tests for `shouldCapture`, header filtering, media grouping (Vitest).
- End-to-end tests with Playwright driving Chromium with the unpacked extension, against the engine's local test server, with a stub native host that records messages.
- A manual capture matrix before each release: direct links, redirects, signed URLs, cookie-protected downloads, `Content-Disposition` names, downloads started by JavaScript, downloads in iframes, Alt-bypass, private windows.
