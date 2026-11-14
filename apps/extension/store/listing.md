# Chrome Web Store listing

Everything the Chrome Web Store asks for, for the Boltget extension (`@boltget/extension`, version 0.3.0). The images are built by `pnpm --filter @boltget/extension store-images` (see [Images](#images)). Re-read the store's current program policies before each submission ([docs/RELEASE.md](../../../docs/RELEASE.md)).

## Store listing tab

**Name:** Boltget

The name comes from the manifest (`_locales/en/messages.json`, `extension_name`).

**Summary** (the manifest's description, `extension_description`; 100 of 132 characters):

> Sends your downloads to the Boltget download manager, with everything it needs to fetch them faster.

**Category:** Productivity › Tools

**Language:** English

**Homepage URL:** https://boltget.app

**Support URL:** https://github.com/boltget/boltget/discussions

**Description:**

```text
Boltget for Chrome hands the downloads you start in the browser to the Boltget app, which fetches them over several connections at once and resumes them where they stopped.

It needs the Boltget app on your computer. Boltget is available for Windows, with macOS and Linux to follow: https://boltget.app/download

WHAT GOES TO BOLTGET
• Big files and the types you choose (archives, disk images, installers, video and audio by default) go to Boltget. Small files and documents stay in the browser.
• Files behind a sign-in come along: with site access, the site's cookies and request headers go with each download, so Boltget fetches the same file the browser would.
• If Boltget is not running, it starts when a download arrives. If it is missing or cannot take a link, such as a one-time link, the browser simply carries on with the download.

YOUR RULES
• Set the smallest file to send, and the file types.
• List the sites to never send from, such as your bank, and the sites to always send from.
• Choose whether downloads from private windows go to Boltget.
• Hold Alt while clicking a link to keep it in the browser, or Alt+Shift to send it to Boltget whatever the rules say.

ONE CLICK AWAY
• The toolbar button shows whether Boltget is connected, switches sending off and on, and pauses it for ten minutes.
• Right-click a link and choose Download with Boltget, or send every link in a selection or a page to Boltget's add sheet to pick the ones you want.

PRIVATE BY DESIGN
• The extension talks only to the Boltget app on your own computer, through the browser's native messaging. It has no server, no account, no analytics and no ads, and loads no remote code.
• Site access is optional and asked for once, with a plain explanation. Without it, downloads still go to Boltget, just without cookies.
• Cookies are passed to the app for the downloads it takes, and only those. They are kept in memory, never written to your download list.

Boltget is free, open-source software under the GPL. Privacy policy: https://boltget.app/privacy/
```

## Privacy practices tab

**Single purpose:**

> Send the downloads the user starts in the browser to the Boltget download manager on the same computer, with the cookies and headers it needs to fetch them.

**Permission justifications** (the permissions of the production manifest, `.output/chrome-mv3/manifest.json`):

| Permission                            | Justification                                                                                                                                                                                                                                                                                                           |
| ------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `downloads`                           | To see a download as it starts, decide by the user's rules whether it goes to Boltget, and once the Boltget app has accepted it, pause and erase the browser's copy so the file is not fetched twice. The menu item Download with Boltget also falls back to a normal browser download when the app cannot take a link. |
| `nativeMessaging`                     | The only way the extension talks to the Boltget app installed on the same computer: it hands over each download and asks the app whether it is running. No other host is contacted.                                                                                                                                     |
| `cookies`                             | To read the cookies for a download's link (and only for downloads handed to Boltget), so the app can make the same signed-in request the browser would. Cookies go only to the local app. Only works on sites the user has granted access to.                                                                           |
| `webRequest`                          | To observe (never block or change) the request headers the browser sent for a download, such as `User-Agent` and `Referer`, so the app's request matches the browser's. Kept in memory for at most 30 seconds, for downloads only.                                                                                      |
| `contextMenus`                        | For the menu items Download with Boltget (on a link), Download selected links with Boltget (on a selection) and Download all links with Boltget (on a page).                                                                                                                                                            |
| `storage`                             | To keep the user's settings in the browser's extension storage: whether sending is on or paused, the smallest file size, file types, the never and always site lists, private windows and the Alt key setting.                                                                                                          |
| `scripting`                           | To collect the links of a selection or page when the user picks one of the menu items for it (a function injected into that tab only at that moment), and to register the small script that notices Alt and Alt+Shift on links, only after the user has granted site access.                                            |
| `activeTab`                           | Lets the link collector run in the tab whose menu the user just used, without needing access to every site for it.                                                                                                                                                                                                      |
| Optional host permission `<all_urls>` | Not granted at install. Asked for once on the settings page, with an explanation, because the cookies and headers of a download are needed on whichever site the user downloads from. The user can remove it on the same page; the extension keeps working without it, minus cookies and the Alt keys.                  |

**Are you using remote code?** No, I am not using remote code. All code is bundled in the package; the app sends capture rules as data, never code.

**Data usage:** the extension does not collect data for its developer. It hands some data to the Boltget app on the user's own computer, which the store's form may count as handling, so tick these, conservatively:

| Category                            | Ticked | What and why                                                                                                                                                                                       |
| ----------------------------------- | ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Personally identifiable information | No     |                                                                                                                                                                                                    |
| Health information                  | No     |                                                                                                                                                                                                    |
| Financial and payment information   | No     |                                                                                                                                                                                                    |
| Authentication information          | Yes    | The site's cookies for a download's link, passed to the local Boltget app so it can fetch a file that needs the user to be signed in. Only with site access, only for downloads handed to Boltget. |
| Personal communications             | No     |                                                                                                                                                                                                    |
| Location                            | No     |                                                                                                                                                                                                    |
| Web history                         | Yes    | The address of the page a download was started from (`Referer`), passed to the local app with that download. No browsing history is read or kept.                                                  |
| User activity                       | No     | The extension notices only whether Alt or Alt+Shift is held when a link is clicked; nothing is recorded or sent.                                                                                   |
| Website content                     | Yes    | The download's link, file name, size and type, and the links of a selection or page when the user asks to send them, passed to the local app.                                                      |

Certify all three:

- I do not sell or transfer user data to third parties, outside of the approved use cases.
- I do not use or transfer user data for purposes that are unrelated to my item's single purpose.
- I do not use or transfer user data to determine creditworthiness or for lending purposes.

**Privacy policy URL:** https://boltget.app/privacy/ (its "The browser extension" section; drop "(coming soon)" from that heading once the listing is live).

## Images

All PNG, sRGB, metadata stripped and checked by `store/scripts/verify.mjs`. Screenshots and promo tiles have no alpha channel; the icon keeps its transparent padding.

| File in `store/output/`    | Size                                                                   | Where it goes                                               |
| -------------------------- | ---------------------------------------------------------------------- | ----------------------------------------------------------- |
| `store-icon-128.png`       | 128 × 128 (the brand's native 96 px Onyx icon in 16 px of clear space) | Store listing › Graphic assets › Store icon                 |
| `screenshot-1-handoff.png` | 1280 × 800                                                             | Screenshots, 1st: downloads from the browser go to the app  |
| `screenshot-2-resume.png`  | 1280 × 800                                                             | Screenshots, 2nd: resume, the file map of a paused download |
| `screenshot-3-rules.png`   | 1280 × 800                                                             | Screenshots, 3rd: the rules and site lists                  |
| `screenshot-4-toolbar.png` | 1280 × 800                                                             | Screenshots, 4th: the toolbar popup                         |
| `screenshot-5-private.png` | 1280 × 800                                                             | Screenshots, 5th: site access and the connection            |
| `promo-small-440x280.png`  | 440 × 280                                                              | Small promo tile                                            |
| `marquee-1400x560.png`     | 1400 × 560                                                             | Marquee promo tile                                          |

What is in them, so the listing stays truthful: every UI is a capture of the real thing, never drawn. The popup and settings page are the release build (`wxt build`) in Chromium, connected to the end-to-end stub host answering as Boltget 0.3.0, with site access granted; the copy only gains the development key (for a known ID) and site access as a granted permission. The app is its real UI from `pnpm --filter @boltget/desktop dev`, running on its simulated engine (`apps/desktop/src/lib/engine/mock.ts`), the same file names as the website's demo. The background is the brand's pearl sky (`brand/source/sky-master.webp`); no image was generated for the listing. The context menu is described but not shown: a script cannot capture the browser's own menus, and the listing shows no imitation browser chrome.

`captures/` keeps the raw captures, so `pnpm --filter @boltget/extension store-images --compose` rebuilds the slides without a browser run. A full run (without `--compose`) needs Playwright's Chromium (any cached build will do, or set `BOLTGET_STORE_BROWSER`), takes about a minute, and on Windows briefly registers the stub host for Chromium and Chrome for Testing under `HKCU` (only where no registration exists) and removes it again. The layouts are in `store/scripts/compose.mjs` and `store/templates/slide.css`.

## How to submit

1. **Build the package.** `pnpm --filter @boltget/extension zip` writes `apps/extension/.output/boltget-extension-0.3.0-chrome.zip` (and the Firefox packages). Check that its `manifest.json` has no `key` field: only development builds carry one, and the store refuses a package with it. The store keeps the extension's real key itself.
2. **Create the item.** In the Developer Dashboard, add a new item and upload the zip. Do not submit for review yet.
3. **Record the ID.** The dashboard shows the item's ID right away. Put it in `packages/native-messaging/extensions.json` as the first entry of `release.chromium`, replacing the placeholder (done 2026-11-14: `aeobielfcjcidnadamggpnomipkijcbj`), and ship a Boltget release whose native host registration lists it. Until that release is out, the store extension cannot reach the app (the popup says "Can't reach Boltget"), and neither can a reviewer.
4. **Fill in the listing** from this file: the description, category and language; the images per the table above; homepage and support URLs.
5. **Fill in Privacy practices** from this file: single purpose, one justification per permission, remote code No, the data usage boxes and certifications, the privacy policy URL.
6. **Test instructions** (Distribution or the review notes): "Install Boltget from https://boltget.app/download, start it once, then download a file larger than 1 MiB, such as https://download.blender.org/release/Blender4.2/blender-4.2.3-windows-x64.msi. The popup shows Connected, and the download appears in the Boltget app instead of the browser."
7. **Visibility.** Unlisted until v1.0 ([docs/BROWSER-INTEGRATION.md](../../../docs/BROWSER-INTEGRATION.md#store-submission)), then submit for review.
8. **After approval,** link the listing from boltget.app and the README, and update the privacy page heading.
