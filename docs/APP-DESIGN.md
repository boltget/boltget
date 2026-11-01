# Desktop app design (v0.2)

The design brief for `apps/desktop`: what the main window, the add flow, the details pane, the tray, notifications and settings look like and why. It applies [DESIGN-PRINCIPLES.md](DESIGN-PRINCIPLES.md) and [brand/BRAND.md](../brand/BRAND.md) to the app; where those documents were silent or disagreed, the decision is recorded here and the other document points to it.

The goal in one line: **it should feel like a premium Windows app, and it should never look complicated.** BoltGet does more than IDM; it must look like it does less.

## Research behind it

Two studies were done on 2026-10-31 before any app code was written:

- **A teardown of 15 download managers and their neighbours:** IDM, Free Download Manager, JDownloader, Motrix and Motrix Next, Gopeed, AB Download Manager, Neat, XDM, Persepolis, Folx, Downie, qBittorrent, Transmission, Fragments, and the download UIs of Chrome, Edge, Safari, Arc and Firefox. It covered their layouts, add flows, progress and segment views, tray behaviour, notifications and settings, plus what their users praise and complain about.
- **What makes desktop apps feel premium,** and how to get that feeling in Tauri 2 on WebView2 under Windows 11. The study looked at Linear, Raycast (including Raycast for Windows), Arc, Things 3, Superhuman, 1Password 8, Zed, Warp, Spotify, the Windows 11 inbox apps and Fluent 2.

What they found, in short:

**What makes download managers look complicated or dated**
- One window per download, then a "complete" dialog on top of it. This is IDM's signature, and the single most dated pattern.
- A form before any value: category, folder and description are asked before the file has been probed, so nothing is known yet (IDM's "Download File Info").
- Hidden staging: JDownloader's LinkGrabber swallows pasted links into a second tab.
- Toolbars that mirror the context menu: 11 to 15 buttons, half of them disabled at any moment, some of them marketing ("Tell a Friend").
- Engine internals shown to users: GID columns, raw HTTP codes, tables of hundreds of advanced keys.
- Wide 8 to 14 column tables that sort live and jitter.
- Status in loud colour everywhere, 3D icons and inconsistent padding. AB Download Manager users name its "padding and margins" bugs.
- Mobile UI on the desktop: floating action buttons, spring-bounce modals.
- Simplifying by removing features. FDM dropped clipboard monitoring and multi-link import to look clean, and long-time users left.

**What makes the best ones feel premium**
- Zero mandatory decisions on add (Downie, browsers).
- Probe first, then ask (FDM's second step).
- One row tells the whole story (browser download rows, Transmission's two-line rows).
- All depth in one place (Transmission's inspector, qBittorrent's properties pane).
- Progress visible outside the window: taskbar, tray.
- Native behaviour everywhere the OS already draws something.
- Restraint: calm grey surfaces, hairline borders, one accent, weight and grey levels instead of boxes, motion of 100 to 250 ms that only explains changes, and every action answered within about 50 to 100 ms.

The full reports, with sources, are kept outside the repository. This document records the decisions taken from them.

## Decisions

### Window

1. **Three regions and no toolbar.** The window has a title bar, a slim sidebar and the list, plus an optional details pane on the right.
   - The title bar is the only chrome. It holds the mark, search, a single primary button ("Add download") and the window controls.
   - There is no toolbar row and no status bar.
   - Actions that depend on the selection appear on the row itself (on hover or keyboard focus), in the native context menu and in `Ctrl+K`.
2. **The sidebar holds views, not a tree.**
   - The views are All, Active, Completed and Scheduled, each with a count.
   - Below them come the categories by file type (Video, Music, Archives, Programs, Documents, Other), then the queues.
   - Under about 900 px of window width the sidebar collapses to an icon rail.
3. **Rows, not columns.**
   - **Comfortable density (40 px rows)** shows two lines. Line 1: file-type icon, name, and the state or primary action at the right. Line 2: a thin progress bar and one muted status line, for example `212 / 420 MiB · 48.2 MB/s · 8 s left`.
   - **Compact density (32 px rows)** puts everything on one line, with the progress bar as a 2 px underline.
   - There are no column headers. A sortable table layout for IDM habits is deferred and will be added only if users ask for it (see [Open questions](#open-questions)).
4. **The list is grouped, and rows never move on their own.**
   - Inside "All", rows are grouped as Active, then Queued and paused, then history by day ("Earlier today", "Yesterday", "This week").
   - A row that is hovered or focused never moves. A finished download moves into history only when the user is not interacting with the list.
   - Sorting happens only on request.
5. **The empty state is the add flow.**
   - One sentence ("Paste a link or drop one here."), the `Ctrl+V` keycap, and the pixel mark played once.
   - When the clipboard holds a URL as the window gains focus, a one-click chip offers it: "Download `file.iso` from example.com".

### Adding a download

6. **One sheet, probe first, one decision.**
   - "Add download", `Ctrl+V` in the window, a dropped link and a CLI handoff all open the same sheet inside the window (not a separate OS window).
   - The sheet probes the link at once and shows what it finds: name, size, type, host and whether the download can resume.
   - The visible fields are the file name and the folder. The folder defaults to the category's folder, with the last three folders offered.
   - `Enter` downloads.
   - "More" holds connections, the speed limit, queue, start later, checksum, sign-in, headers and referrer.
   - A skeleton appears only if the probe takes longer than 150 ms.
7. **The sheet warns before a bad download, not after.** It warns when:
   - the link returns a web page instead of a file ("This link returns a web page, not a file. It may need a sign-in.");
   - the file was downloaded before ("You downloaded this on Oct 12. Open it, or download again");
   - the server allows a single connection only and resume is unavailable;
   - there is not enough disk space.

   AB Download Manager's most common complaint is downloads that turn out to be `.html` pages; the engine's probe already knows enough to catch this.
8. **Several links become a visible checklist in the same sheet.** It lists the probed name and size of each link, a select-all checkbox, one folder for all, and the button "Download 12 files". Links never land in a hidden staging tab.
9. **Clipboard monitoring is opt-in and only offers.** When it is on, a copied link appears as an in-app chip (or, if the user enables it, a toast). Repeats collapse, and links copied from BoltGet itself are ignored. Nothing is ever added silently.

### Details

10. **One details pane (`Ctrl+I`), never a window per download.**
    - The top of the pane repeats the website's hero card: large tabular stats (speed, received, time left, connections), the **file map** and a throughput sparkline.
    - Below it are three sections:
      - Connections, including events such as "took half of #3";
      - File: path, URL, checksum with copy buttons, server and ETag;
      - Log, which is the only place HTTP codes appear.
11. **The file map explains; it does not decorate.**
    - Hovering a square shows its byte range and the connection that wrote it.
    - When a segment splits, the run of squares visibly divides.
    - When the download finishes, the map settles into one calm field, and "SHA-256 verified" appears as a quiet note.
    - The map is drawn on a canvas at 30 fps or less.
12. **Live controls in the pane.** The pane has a connections stepper and a per-download speed limit, both of which apply immediately. "Refresh link…" appears when a link has expired.

### Tray, background and notifications

13. **The tray is a status surface, not a second app.**
    - Left click opens or focuses the window.
    - Right click opens a native menu: Add from clipboard, Pause all / Resume all, Speed limit ▸, Open downloads folder, Quit.
    - The tooltip reads `3 active · 48.2 MB/s · 2 min left`.
    - The tray mark animates only while bytes are flowing, slowly (one cycle per 2 to 3 s). It freezes when paused or stalled, and stays static under reduced motion.
14. **Progress shows outside the window.**
    - The taskbar button shows the total progress of active downloads, yellow when paused and red on error (Tauri's `set_progress_bar`).
    - The jump list offers "Add download from clipboard", "Pause all" and "Open downloads folder".
15. **The window closes to the tray by default.** This is explained once, on the first close. After some minutes hidden, the WebView is destroyed; the engine, the tray and notifications keep running, and the window is rebuilt in under 300 ms when it opens. Motrix Next does the same on Tauri 2.
16. **Notifications use native Windows toasts only, and there are three kinds:**
    - *Completed*: "420 MiB in 7.9 s", with the actions Open and Show in folder.
    - *Needs attention*: link expired, disk full or sign-in required, with the action Fix…
    - *Batch summary*: shown when three or more downloads finish within a minute.

    There is never a modal dialog. No toast appears for anything the user just did in a focused window, and Focus sessions are respected.

### Settings and first run

17. **Settings open in their own window (`Ctrl+,`).**
    - The window has search at the top and at most seven sections: General, Downloads, Connections and speed, Network, Browser (v0.3), Shortcuts and Advanced.
    - Each setting is a row in the Windows 11 style: a label, a one-sentence description and the control on the right.
    - Changes apply immediately; there is no Save or Apply button.
    - Settings search also feeds `Ctrl+K`.
18. **There is no wizard and no tour.**
    - The first window is the empty state, with one dismissible line: "Downloads go to `C:\Users\…\Downloads`. Change".
    - Opt-ins are asked the first time they matter. Clipboard monitoring is offered the first time a copied link is seen. Close to tray is explained on the first close.
19. **Trust is visible but quiet.** "About" shows the version, the licence and "No ads, no tracking, no bundled offers". The updater shows the changelog and never installs anything else.

### Keyboard

20. **`Ctrl+K` reaches every action, every setting and every download.**
    - Each result shows its shortcut.
    - Synonyms match ("throttle" finds "Limit speed").
    - `Enter` runs the result; `Tab` opens it in context.
    - The default shortcuts are in [DESIGN-PRINCIPLES.md](DESIGN-PRINCIPLES.md#6-keyboard-first).
21. **Quick add (stretch goal for v0.2).** A global hotkey opens a small centred window prefilled from the clipboard; `Enter` downloads and `Esc` closes. This works like Things' Quick Entry and Raycast.

### Visual system

22. **One canvas, hairlines, almost no shadows.** Structure comes from 1 px borders and one step of surface lift. Shadows appear only on things that float: menus, the palette, toasts and the sheet.
23. **Neutral by default, emerald only for "alive".**
    - Emerald is used only on active progress, the primary button, the focus ring and the selected view. An idle list is almost entirely grey.
    - Status colour stays small: a glyph or the progress fill carries it, and the text stays neutral. Error rows get no red background.
24. **Hover and press feedback:**
    - Hover is a 4% overlay with no fade in and a 100 ms fade out.
    - Press adds another 4% step, never a scale.
    - Selection is an accent tint with a 2 px indicator on the left.
25. **The app's type scale.** Text is 13 px (rows and controls), 12 px (secondary text, weight 500 on dark) and 15 px (pane titles). Larger sizes are reserved for empty states and settings headers. Weights are 400 and 500, with 600 only for titles. Live numbers use Geist with tabular figures; Geist Mono is used for URLs, hashes, headers and the log.
26. **Corner radii:** controls 4 px, floating surfaces 8 px, the window 8 px (the system's rounded corners). Anything that *is* a chunk (progress fill, file map, the mark) stays square, at radius 0.
27. **Motion.**
    - Small changes take 100 to 150 ms; panes and sheets take 200 to 250 ms.
    - Elements enter on `cubic-bezier(0, 0, 0, 1)` and leave in about two thirds of the entering duration on `cubic-bezier(0.33, 0, 0.67, 1)`.
    - Motion driven by data is linear: the progress fill interpolates across each 250 ms tick with `transform`, never `width`.
    - Digits never tween. Speed updates at 1 to 2 Hz as a 3 s moving average, and the time left changes only when it moves by more than 10%.
    - Nothing animates on first paint.
28. **No splash screen.** The window opens with real data in its first frame; the list is local SQLite, so it needs no spinners and no skeletons. The signature animation appears in exactly two places in the app: the empty state (played once) and the tray mark while active.

### Native on Windows

29. **Native wherever the OS draws something better.** That means context menus and the tray menu (Tauri `Menu::popup`), file dialogs, toasts, the taskbar progress bar, overlay scrollbars (WebView2 `fluentOverlay`) and the title bar's snap layouts.
30. **Avoid the "web app in a window" tells:**
    - **Launch:** no white flash. Set `backgroundColor`, keep the window hidden until the first frame, and inline the background CSS.
    - **Pointer and text:** no text selection on chrome and no I-beam over labels. The arrow cursor stays on buttons (the Windows convention).
    - **Focus:** rings appear only for keyboard focus (`:focus-visible`), as a 2 px emerald ring with a 2 px offset.
    - **Browser leftovers:** release builds have no browser right-click menu, and no reload, zoom or devtools shortcuts.
    - **Window state:** size and position are restored before the window is shown, and a missing monitor is handled.
    - **Hidden window:** while the window is hidden it receives no progress ticks; one snapshot arrives when it is shown again.
31. **Title bar.** It is custom, with the snap layout flyout kept: a small native window over the maximize button returns `HTMAXBUTTON` (see Tauri [#4531](https://github.com/tauri-apps/tauri/issues/4531)). The caption buttons are 46 px wide and use Segoe Fluent Icons glyphs. If that work slips, v0.2 falls back to native decorations with a themed caption colour, because a title bar without snap layouts is worse than a plain one.
    - *Built (2026-11-01):* the overlay works on Windows 11, so the fallback is not used. The buttons fill the title bar's height (46 × 44 px, like Edge and Terminal) rather than 46 × 32, so the top edge of a maximized window still hits them. The overlay forwards hover and press to the page, which draws the button, and maximizes or restores on click; keyboard and screen readers use the page's button. Tauri's own top-edge resize strip stays above the overlay. Glyphs fall back to Segoe MDL2 Assets (Windows 10), then to inline SVG.
32. **Mica is optional and confined to the chrome.** If used at all, it sits behind the title bar and sidebar on Windows 11, never under the list. It is off when the system turns transparency off. The content area is always the solid theme background.
33. **Performance budgets.**
    - The list is virtualised with a fixed row height.
    - Progress ticks are coalesced and applied to per-row fine-grained state.
    - 500 rows with 20 active scroll at 60 fps with no long tasks over 50 ms.
    - Pause, resume and remove update the row optimistically within 50 ms.
    - The window is visible with real data in under 400 ms on a warm start.

### Direction: Daylight

34. **The light theme is designed first.** Three directions were prototyped on 2026-10-31 with the same layout and live data: Ink (the brand's near-black), Daylight (light, like the website and its hero card) and Mica (Windows 11 tinted chrome). The owner chose **Daylight**:
    - The app looks like the website's hero card, so the first screenshot matches the marketing.
    - Windows ships in light mode by default.
    - The tokens come from `brand/tokens/tokens.css`, light theme. The chrome (title bar and sidebar) is `#F6F6F7`, the content area white, hairlines `rgb(9 9 11 / 8%)`, and the accent `#047857` with white text. Progress fills use the light gradient (`#34D399` → `#047857`).
    - Dark (Ink) remains a full theme. The app follows the system setting and is reviewed in both themes. Light is the reference for screenshots, and it is designed and polished first.
    - Mica is not used in v0.2.

## The app shell as built

The first step of v0.2 (2026-11-01) built the shell that later steps fill in: title bar, sidebar, the empty state and the theme plumbing. Decisions it took where this brief was open:

- **The content area is a sheet on the chrome.** The title bar and sidebar are one grey surface; the content area sits on it with a hairline on its top and left edges and an 8 px top-left corner, like the website's hero card.
- **Ink's surfaces.** In dark, the content is the brand's `#0B0B0F` and the chrome one step lighter (`#141418`), the inverse of Daylight's grey chrome around white content. Secondary 12 px text is weight 500 in dark (#25).
- **The colours are generated.** Every app colour is a semantic token (`--color-chrome`, `--color-hairline`, `--color-accent-tint`, …) defined once, for both themes, in `packages/ui/scripts/generate.mjs` from the brand tokens; see [TECH-STACK.md, ADR-004](TECH-STACK.md#adr-004-svelte-5-typescript-and-vite-for-the-ui). Daylight's chrome comes out at exactly `#F6F6F7`. The close button's hover red is the brand's error red (`#DC2626`), not Windows' own `#C42B1C`, so no colour is hard-coded.
- **Launch.** The window is created hidden, its background set from Rust to the theme's content colour while it is still hidden, and the page inlines the same colour in `index.html`. The page applies the theme, waits for the bundled fonts and one frame, then asks Rust to show the window. If the page never asks, Rust shows it after 3 s. Size, position and the maximized state are restored while it is hidden.
- **The title bar's icon** is the pixel-snapped app icon (`icon-16.svg` at 1x and 2x, `icon-24.svg` at 1.5x), since the mark without a tile has a 24 px minimum.
- **The empty state's mark** is drawn at 108 px, where the canonical squares land on whole pixels at 100% scale. It waits, dim, until the window is visible, then plays once.
- **The icon rail** is 52 px wide; labels become tooltips.
- **WebView2:** no browser context menu, status bubble, pinch zoom or swipe navigation in any build; no browser accelerator keys (reload, find, print, devtools) in release builds; overlay scrollbars (`fluentOverlay`).
- **Reviewing themes.** Debug builds accept `BOLTGET_DEV_THEME=light|dark` and flip the theme with `Ctrl+Shift+L`; the Theme setting itself arrives with the settings window.

## Resolved conflicts in the design documents

The research found nine places where the existing documents disagreed or did not fit a desktop app. All were resolved as follows, and the documents concerned were updated:

| Topic | Before | Decision |
|---|---|---|
| Primary button | DESIGN-PRINCIPLES allowed the gradient; BRAND forbade it on buttons | Solid emerald button. The gradient stays on the mark and the active progress fill |
| Numbers | BRAND: all numbers in Geist Mono | Geist with tabular figures in lists; Mono for URLs, hashes, headers, the log |
| Type scale | BRAND's 15 px body (a website scale) | The app scale above (13/12/15); BRAND keeps its scale for the web |
| Focus ring colour | DESIGN-PRINCIPLES suggested the OS accent | Always emerald; system colours only in high contrast (`forced-colors`) |
| Splash screen | DESIGN-PRINCIPLES allowed a short launch splash | No splash |
| Shadows | BRAND: "no drop shadows" | Applies to the mark and pixels; floating UI uses elevation tokens |
| Square vs rounded | Undefined for app controls | Chunks are square; controls and containers follow Windows 11 (4/8 px) |
| Mica vs ink | Undefined | Content is always solid; Mica at most behind the chrome |
| Hover timing | 120–200 ms for small transitions | Hover in and press are instant; hover out 100 ms |

## Open questions

These decisions are settled with the clickable prototype, before the app is built:

- **Auto-start on paste.** Should `Ctrl+V` in the window start the download at once, with an undoable preview row (Downie and browsers), or open the sheet with Download focused (one `Enter`)? The default above is the sheet; auto-start is the alternative to test.
- **A table layout for IDM users,** or one row model only. Rows ship in v0.2; a table is added later if users ask.
- **Segments in the row.** Should the progress bar of an active row hint at gaps (a 1 px mini map), or should segments appear only in the details pane?
