# Browser integration

The Boltget extension catches downloads in the browser and hands them to the Boltget app together with everything the app needs to fetch the same file (cookies, referrer, headers). It also detects media on pages and offers to send the page to the app. It does no downloading itself and contains no site-specific extraction logic.

- Code: `apps/extension` (`@boltget/extension`); the native host is `crates/native-host`, its protocol `crates/ipc/src/browser.rs`
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

> **As built (v0.3 step 2).** `srcDir` is the package root and WXT's auto-imports are off (imports are explicit, as in the app):
>
> | Path | What |
> |---|---|
> | `wxt.config.ts` | Manifest per browser and flavor (see "Flavors and builds" below) |
> | `entrypoints/background.ts` | The service worker / event page: download listeners, the handoff, context menus, the modifier script's registration, `hello` at start-up |
> | `entrypoints/modifiers.ts` | The modifier-key watcher, an *unlisted* script registered at run time (a WXT content script would add its match patterns to the required host permissions) |
> | `entrypoints/popup/`, `entrypoints/options/` | Svelte 5 pages built from `@boltget/ui`'s tokens, fonts and primitives; `lib/ui/` holds what they share |
> | `lib/capture.ts` | `shouldCapture` and the type matching |
> | `lib/handoff.ts` | Pause, ask, then cancel and erase or give back (the fallback guarantee) |
> | `lib/native.ts` | The native messaging client, reply checks, the connection states |
> | `lib/headers.ts`, `lib/cookies.ts` | Header observation and the allowlist; cookie collection |
> | `lib/message.ts` | Builds `capture`, `add_links` and `media_page` within the host's limits |
> | `lib/modifiers.ts`, `lib/rules.ts`, `lib/settings.ts` | The click log, rule normalising and site matching, what is stored |
> | `lib/collect.ts` | The link collector injected for the link menus |
> | `lib/platform.ts` | The thin layer over the browser APIs; everything else in `lib/` is pure and unit-tested |
> | `lib/en.json`, `lib/i18n.ts` | The string catalog and `t()` (the app's no-literal-strings ESLint rule applies); the manifest's own strings are in `public/_locales/en/messages.json` |
> | `public/icons/` | Onyx and its Paper twin at 16, 32, 48 and 128 px, copied from `brand/png` by `scripts/icons.mjs` (`check` fails while stale). Firefox swaps them with its theme (`theme_icons`); Chromium has no manifest switch, so the popup and options page report the browser's colour scheme and the background sets the toolbar icon, now and at each start |
> | `e2e/` | Playwright tests, the stub host and its registration |
>
> Media detection (`lib/media.ts`) arrives with v0.4.

## Permissions

| Permission | Why |
|---|---|
| `downloads` | Observe new downloads, pause/cancel/erase the browser's copy after the app accepts it |
| `nativeMessaging` | Talk to the Boltget app |
| `cookies` | Read cookies for the download URL so the app can make the same authenticated request |
| `webRequest` | Observe (not block, on Chrome) request headers such as `Referer` and `User-Agent`, and response `Content-Type` for media detection |
| `contextMenus` | "Download with Boltget", "Download all links", "Send page to Boltget" |
| `storage` | Capture rules and site lists |
| `scripting` | Inject the link collector only when the user invokes "Download all links"; register the modifier-key script once site access is granted |
| `activeTab` | Lets the link collector run in the tab whose menu was used, without host permissions |
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

> **As built (v0.3 step 2): flavors and builds.** `wxt build` is the release flavor (store IDs, host `app.boltget.native_host`, Firefox `extension@boltget.app`). `wxt build --mode development` (and `wxt`, the dev server) is the development flavor: the manifest carries `extensions.json`'s `chromiumKey` as `key`, so an unpacked Chromium build gets the pinned ID `fckefagfbiinebmgfpgjiielmmmaggbd`; it talks to `app.boltget.native_host_dev`, and Firefox runs it as `extension-dev@boltget.app`. `wxt build --mode e2e` is the development flavor for the end-to-end tests: it talks to `app.boltget.native_host_e2e` and holds `<all_urls>` as a required host permission, because a test cannot answer the browser's permission prompt. Firefox builds are Manifest V3 too, with `data_collection_permissions: { required: ["none"] }` and a minimum of Firefox 128; Chromium builds ask for Chrome 120. `pnpm --filter @boltget/extension zip` writes `boltget-extension-<version>-chrome.zip`, `-firefox.zip` and the Firefox sources zip to `.output/`.

### Chromium (Chrome, Edge)

Manifest V3 extensions cannot block network requests with `webRequest`, so capture works through the `downloads` API:

1. `chrome.downloads.onCreated` gives the `DownloadItem` (URL, final URL, referrer, MIME, size if known, incognito flag). `onDeterminingFilename` additionally gives the browser's proposed filename once headers are known.
2. The extension **pauses** the browser download immediately, then asks the app.
3. Only after the app replies "accepted" does the extension cancel and erase the browser's download. If the app is unreachable within a timeout (design target: 3 s including app launch), the download is resumed in the browser.

> **As built (v0.3 step 2).** The extension decides in `downloads.onDeterminingFilename`, not `onCreated`: it fires once per download, after the response headers, with the browser's file name, which the type rules need (at `onCreated` Chromium's `filename` is still empty). The listener calls `suggest()` at once without a suggestion (the browser's name stands; Boltget never renames in the browser), then decides and pauses. The timeout is 20 s, not 3 s (see "Timing" below). On `accepted` the browser's download is cancelled and erased; if the browser had already finished it (the pause failed and the file was small), both copies are kept rather than deleting a finished file. On anything else the download is given back (`giveBack` in `lib/handoff.ts`): resumed when the extension paused it and it is still paused; if resuming fails, or the paused connection was dropped and the download cannot resume, the link is started again with `downloads.download` and the dead entry erased; a download the user cancelled or removed meanwhile is left alone. Downloads started by an extension (`byExtensionId`), Boltget's own restarts included, are never captured. The end-to-end tests check that a refused one-time link finishes from its first and only request.

Caveat: by the time the extension sees the download, the browser has already sent the request. For **one-time links** (a token valid for a single request), the app's request may then be refused. In that case the app replies `rejected` with the reason `link_refused` after its probe fails, and the extension resumes the browser's paused download, which still holds the original response. The user gets the file either way; it just isn't accelerated.

> **Implemented (v0.3 step 1, the app's side; `apps/desktop/src-tauri/src/browser.rs`).** Before it answers a `capture`, the app makes its own first request for the link with the browser's cookies and headers (the engine's probe), within 4 s. A refusal (401, 403, 404, 410, 429 and the like: `link_refused`), any other failure, or no answer in time (`failed`) is a `rejected` reply, and the browser keeps its download: losing a download is worse than not accelerating it. Otherwise the download is added to the main queue in its category's folder and the reply is `accepted` with its id. This costs one extra request per capture (the engine probes again when the download starts); folding the two together is left for later.
>
> **"Ask before adding browser downloads"** (Settings, Browser; off by default, which keeps "zero mandatory decisions on add") opens the add sheet with the link instead, and the reply is `accepted` with `confirm: true` at once: the browser's copy is dropped as the sheet opens, as IDM does with its dialog, because the extension cannot wait for the user. The request context waits in the app's memory, by link, for 30 minutes (at most 5000 links): the sheet's preview and its Download button use it, so the cookies and headers are the browser's. Cancelling the sheet drops the download. `add_links` ("Download all links", a selection's links) always opens the add sheet with the links, which is the picker for now.
>
> Browser cookies are not saved with a download (no secret is stored before DPAPI encryption arrives, [ENGINE.md](ENGINE.md#request-context)). The engine keeps them in memory with the download while Boltget runs, so every request of it carries them: the queue's start, each added connection and split, and every pause and resume. A captured download that is resumed after Boltget restarts runs without them; if the server wants them it moves to "needs a new link", with a message saying the browser's cookies are not kept after Boltget closes and to download it again from the browser. Private-window captures are treated like any other for now (the "never in history" setting is still to do).

### Firefox

Firefox's Manifest V3 implementation still supports blocking `webRequest` (`webRequestBlocking`). The Firefox build can therefore decide in `webRequest.onHeadersReceived` (where `Content-Disposition`, `Content-Type` and `Content-Length` are visible) and cancel the request *before* the browser starts a download, which is cleaner and avoids the double-request problem for most cases. The `downloads` API path is kept as a fallback for downloads that start without a matching observed request (for example `blob:` URLs, which the extension ignores, or downloads initiated by other extensions).

Whether to use blocking `webRequest` on Firefox in v0.3 or keep a single code path is decided during implementation; the capture decision function is shared either way.

> **Decided (v0.3 step 2): one code path.** The Firefox build uses the `downloads` API like Chromium and decides in `downloads.onCreated` (Firefox has no `onDeterminingFilename`; its item already has the full target path, whose last segment is the name). Blocking `webRequest` would need `webRequestBlocking` and required host permissions for every site, against goal 3, and a second capture path to test. Firefox pauses only downloads it can resume; when the pause fails the download simply continues, and on `accepted` it is cancelled then. Revisit if one-time links prove common on Firefox. The Firefox build has not been run in Firefox yet (see "Testing").

### What is never captured

- `blob:` and `data:` URLs (the bytes only exist inside the page; the app cannot fetch them).
- Downloads from incognito/private windows, unless the user has enabled the extension in private windows **and** turned on "capture in private windows" in options. Private downloads never appear in the app's history after completion (opt-in setting).
- `file:` URLs.
- Requests with a `POST` body that the extension cannot see (form-submission downloads). These stay in the browser.
- Downloads started by the Boltget app itself (none in v1, but guarded).

> **As built (v0.3 step 2).** All of the above, with these decisions: a download started by any extension is left alone (that covers Boltget's own fallback restarts and avoids fighting other download extensions); a `POST` is recognised from the request `webRequest` observed, so only with site access (without it, a form download goes to the app, whose own first request fails, and it stays in the browser); an **explicit** request, the link menu or Alt+Shift, is honoured in a private window too, with `incognito: true` in the message, because the user asked for Boltget by name. Automatic capture in private windows still needs "Private windows" in options.

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

> **As built (v0.3 step 2), `lib/capture.ts`.** The precedence is: not fetchable (`blob:`, `data:`, `file:`, over 32 KiB) → started by an extension → `POST` → Alt (bypass) → Alt+Shift or the link menu (forced) → private window without the setting → capture off or paused → never-capture list → always-capture list (any type, any size) → type and size. For the type, the first of file name, final URL's path, link's path and MIME type that names a type decides; extensions that name a script rather than a file (`php`, `asp`, `aspx`, `jsp`, `cgi`, `html` and the like) name nothing, and MIME types map to the listed extensions (`video/mp4` to `mp4`, `application/x-msdownload` to `exe`, ...). A listed type is captured unless its known size is under the threshold; generic bytes (`application/octet-stream` and kin, or no MIME type) with no telling name are captured when their known size reaches the threshold; everything else stays. Site lists match the hosts of the link, the final URL and the referrer as suffixes on label boundaries; the never list wins. A test enumerates every combination of these inputs (41,472) against an independent statement of this precedence.
>
> Rules live in `storage.local`: the app's arrive with each `hello` (at install, at browser start, and whenever the popup or options open) and apply until the user edits them in options; then the user's are kept and the app's are kept aside for "Use Boltget's rules". The popup's switch and "Pause for 10 minutes" are the extension's own and sit on top of the rules' `enabled`.

### Modifier-key bypass

- **Bypass (let the browser download):** hold **Alt** while clicking the link (default; configurable).
- **Force capture:** hold **Alt + Shift** while clicking, regardless of rules.

Implementation: a small content script records `altKey`/`shiftKey` on `mousedown`/`click` for links and posts `{tabId, href, modifiers, t}` to the background, which keeps it for a short window (design target: 2 s). When a download from that tab with a matching URL appears, the recorded modifiers apply.

Notes:

- In Chromium, Alt+click on a link already means "download link" by default, so Alt fits naturally: the user is asking the browser to download it.
- The content script is injected only on pages where host permission was granted. Without it, modifier bypass is unavailable and the popup toggle is the bypass.

> **As built (v0.3 step 2).** The watcher (`entrypoints/modifiers.ts`) is registered with `scripting.registerContentScripts` (all frames, `document_start`, `http`/`https` pages) while the user has granted `<all_urls>` and "Alt and Alt+Shift on links" is on in options, and unregistered otherwise. It reports only a left `mousedown` or Enter with Alt held on a link, as `{href, alt, shift}`. A Chromium `DownloadItem` has no tab, so the background matches a download's link or final URL to a click within 2 s, once.

## Context menus

| Context | Item | Message |
|---|---|---|
| Link | Download with Boltget | `capture` (forced) for the link URL, with cookies for it and the page as referrer |
| Selection | Download selected links with Boltget | Content script collects `<a href>` within the selection → `add_links` |
| Page | Download all links with Boltget... | Content script collects all links → `add_links`; the app opens a picker with filters by type |
| Video / audio element | Download media with Boltget | `media_page` with the page URL and element source hint |
| Page | Send page to Boltget | `media_page` with the page URL |

Media-related items follow the [store policy constraints](#store-policy-constraints).

> **As built (v0.3 step 2).** Link, selection, page and "Send page to Boltget" exist; the video/audio element item comes with media detection in v0.4. The link item sends a forced `capture` with the link's cookies and the page as referrer; if the app does not accept it, the extension starts the download in the browser instead, so the link is never lost. The selection and page items inject `lib/collect.ts` into the frame that was clicked (`activeTab`) and send `add_links` with cookies for each site the links are on (at most 20 sites). "Send page" is answered `unsupported` until v0.4, and the extension says so. On YouTube's domains it sends nothing and says it is not available there; hiding the item there needs the tab's URL and is left to v0.4's media work. A menu outcome that needs words leaves a notice: a badge on the toolbar button and, where the browser allows `action.openPopup`, the popup, which shows it.

## Cookies, headers and referrer forwarding

For each captured download the extension sends:

- **Cookies:** `cookies.getAll({ url })` for the download URL and, if different, the final URL, as structured records (name, value, domain, path, secure, httpOnly, sameSite, expirationDate). For partitioned cookies (CHIPS), the extension also queries with the top-level site's `partitionKey` where the browser supports it. The app builds a cookie jar from these records (see [ENGINE.md, Authentication and cookies](ENGINE.md#authentication-and-cookies)).
- **Request headers** observed in `webRequest.onSendHeaders` for the matching request (Chrome needs the `extraHeaders` option to expose `Referer` and `Cookie`), filtered through the allowlist: `User-Agent`, `Referer`, `Accept`, `Accept-Language`, `Origin`, `Authorization`. Observed headers are kept in a short-lived map keyed by request ID and URL (design target: 30 s, size-capped) and discarded after use.
- **Referrer:** `DownloadItem.referrer`, falling back to the tab URL.
- **Filename hint and size hint** from the browser.

Nothing is sent for downloads that are not captured. Nothing is stored by the extension beyond the short-lived header map.

> **As built (v0.3 step 2).** Cookies: `cookies.getAll({url, storeId})` for the link and the final URL, and again with `partitionKey: {topLevelSite}` built from the referrer's scheme and host (best effort: a browser that refuses the query gives no partitioned cookies); on Firefox with `firstPartyDomain: null` (every first-party domain) and the item's `cookieStoreId`; on Chromium store `0`, or `1` in a private window. Deduplicated and cut to the protocol's 300 cookies and 64 KiB. Headers: `onSendHeaders` for `main_frame`, `sub_frame` and `other` requests (with `extraHeaders` on Chromium), at most 256 requests for 30 s, each taken once. The context menus, which have no observed request, send the browser's `User-Agent` and the page as `Referer`. A value the host would refuse (a file name with a folder or control characters, a non-`http` referrer, a MIME type with non-ASCII) is left out rather than sent, since one bad field refuses the whole message.
>
> **Site access.** `<all_urls>` is an optional host permission. After install the options page opens with a welcome note and explains it in one paragraph (what is read, for which downloads, where it goes), with Allow access; the popup offers it while it is missing. Without it, capture still works, without cookies or headers, and the Alt keys are off.

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
> - `release`: host `app.boltget.native_host`, Chromium IDs `aeobielfcjcidnadamggpnomipkijcbj` (Chrome Web Store, assigned 2026-11-14) and `bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb` (Edge Add-ons), the latter a **placeholder** to replace with the store-assigned ID before the first Edge release (they are well-formed, so browsers accept the manifest, and belong to no extension), and Firefox `extension@boltget.app`.
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

> **As built (v0.3 step 2).** The popup and options page send `hello` each time they open (Test connection sends it again). A host that exits, one that refuses this extension (`forbidden`), a timeout or a reply that is not the protocol's show "Can't reach Boltget". The extension also checks versions itself: a `hello` whose `oldest` is above its version is Extension too old, one whose `current` is below its oldest is App too old.
>
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

> **As built (v0.3 step 2).**
>
> - **Vitest** (`pnpm --filter @boltget/extension test`): everything in `lib/` but `platform.ts`, including the exhaustive `shouldCapture` table; the handoff against a fake `downloads` API (app missing; hung past the timeout, with a late `accepted` that changes nothing; every refusal; a one-time link whose paused connection dropped; a download the user cancelled or removed; a pause that failed); and a check that the default file types are `DEFAULT_EXTENSIONS` in `crates/ipc/src/browser.rs`.
> - **Playwright** (`pnpm --filter @boltget/extension e2e`): builds `--mode e2e`, registers `e2e/stub-host.mjs` as `app.boltget.native_host_e2e` (on Windows HKCU keys for Chromium, Chrome for Testing, Chrome and Edge; on Linux the per-user folders and the profile's own; removed after each test), and loads the build into Chromium (new headless mode) with a fresh profile whose downloads go to a temporary folder. Playwright's own download handling skips the extensions' `onDeterminingFilename`, so the tests give downloads back to the browser (`Browser.setDownloadBehavior: default`). A local server serves slow files, a one-time link and a cookie. The tests cover capture with cookies and headers, generic bytes and Alt+Shift; what stays (small files, documents, `blob:`, Alt, capture off, the never list, the app's rules from `hello`); the fallback when the app refuses a one-time link (finished from its single request), is not installed, exits, answers garbage or hangs (resumed after 20 s); and the popup and options pages. `BOLTGET_E2E_BROWSER` runs another Chromium build. CI runs them on Windows (`.github/workflows/extension.yml`). The context menus are not driven: Playwright cannot click the browser's own menus.
> - **Real run (2026-11-14, by hand):** the development build in Microsoft Edge (temporary profile, `--load-extension`), the debug `boltget-native-host` registered with `--register`, and the debug desktop app with its own data and download folders. A 4 MiB archive went to the app, which the host started from cold; the app accepted it and downloaded it byte for byte while the browser's copy was erased. A one-time link was refused by the app's first request (410, `link_refused`) and finished in the browser. With site access, the app's requests carried the browser's cookie and `User-Agent`. The run found the host starting the command-line tool (`boltget.exe`) for `Boltget.exe` in a build folder, now fixed in the host.
> - **Not yet run:** the Firefox build in Firefox, and the manual matrix on all three browsers.

## Development

1. Build and register the development host: `cargo build -p boltget-native-host -p boltget-desktop`, then `target\debug\boltget-native-host.exe --register` (a debug host registers `app.boltget.native_host_dev` for Chrome, Edge and Firefox with the development IDs; `--status` checks, `--unregister` removes). The host starts `boltget-desktop.exe` from its own folder when a download arrives; `BOLTGET_NATIVE_HOST_APP` starts another build, and `BOLTGET_DATA_DIR` in the browser's environment (the host and the app inherit it) keeps test downloads apart from your own.
2. Build the extension: `pnpm --filter @boltget/extension build:dev` (Chrome and Edge, `.output/chrome-mv3-dev`) or `build:firefox:dev` (`.output/firefox-mv3-dev`). `pnpm --filter @boltget/extension dev` rebuilds on change and opens a browser with it loaded.
3. Chrome or Edge: `chrome://extensions` (or `edge://extensions`), turn on Developer mode, Load unpacked, choose `.output/chrome-mv3-dev`. The ID must read `fckefagfbiinebmgfpgjiielmmmaggbd`; the manifest's key pins it.
4. Firefox: `about:debugging#/runtime/this-firefox`, Load Temporary Add-on, choose `.output/firefox-mv3-dev/manifest.json`. It runs as `extension-dev@boltget.app` until Firefox restarts.
5. A separate browser profile for development is best. The development extension and a released one can sit side by side: they talk to different hosts.

## Store submission

What the owner has to do before the first submission (the packages build with `pnpm --filter @boltget/extension zip`):

> **As built:** the Chrome Web Store listing is ready in `apps/extension/store/`: `listing.md` holds the text, the single purpose, the permission justifications, the data-use answers and a submission checklist; `pnpm --filter @boltget/extension store-images` captures the real popup, settings page and app UI and composes the screenshots, promo tiles and store icon into `store/output/`, metadata stripped and verified.

- **Accounts:** a Chrome Web Store developer account (one-time registration fee, verified contact email), a Microsoft Partner Center account for Edge Add-ons, and a Firefox Add-ons (AMO) developer account, each with two-step verification.
- **IDs:** after the first Chrome and Edge uploads, put the store-assigned IDs in `packages/native-messaging/extensions.json` (`release.chromium`, replacing the `aaaa...` and `bbbb...` placeholders) and ship a Boltget whose host registration lists them. Firefox keeps `extension@boltget.app`.
- **Listing:** the single purpose ("send downloads to the Boltget download manager"), a description, screenshots of the popup and options page (1280 × 800 for Chrome), the 128 px icon from `public/icons`, and a promotional tile if wanted. Nothing about YouTube.
- **Permission justifications:** one line each, from the Permissions table above (`downloads`, `nativeMessaging`, `cookies`, `webRequest`, `contextMenus`, `storage`, `scripting`, `activeTab`, and the optional `<all_urls>`).
- **Privacy disclosures:** Chrome's data-use form and Edge's equivalent. The extension handles website content and authentication information (cookies) only to hand a download to the Boltget app on the same computer; nothing is sold, transferred to third parties or used for anything else, and there is no remote code. A privacy policy URL: boltget.app's privacy page, updated to cover the extension. Firefox: `data_collection_permissions` says `none` (nothing leaves the computer).
- **Firefox sources:** AMO asks for the source of a bundled extension. The generated sources zip lacks the workspace packages it imports (`packages/ui`, `packages/native-messaging`, `brand/`), so upload a tagged archive of the repository with the build steps instead (`pnpm install`, then `pnpm --filter @boltget/extension build:firefox`).
- **Review:** re-read each store's current policies first ([RELEASE.md](RELEASE.md)). The listings can stay unlisted until v1.0.
