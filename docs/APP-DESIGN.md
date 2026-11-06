# Desktop app design (v0.2)

The design brief for `apps/desktop`: what the main window, the add flow, the details pane, the tray, notifications and settings look like and why. It applies [DESIGN-PRINCIPLES.md](DESIGN-PRINCIPLES.md) and [brand/BRAND.md](../brand/BRAND.md) to the app; where those documents were silent or disagreed, the decision is recorded here and the other document points to it.

The goal in one line: **it should feel like a premium Windows app, and it should never look complicated.** Boltget does more than IDM; it must look like it does less.

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
9. **Clipboard monitoring is opt-in and only offers.** When it is on, a copied link appears as an in-app chip (or, if the user enables it, a toast). Repeats collapse, and links copied from Boltget itself are ignored. Nothing is ever added silently.

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

17. **Settings open in their own window (`Ctrl+,`).** *(Built as a screen of the main window instead; see [Settings, the palette and shortcuts as built](#settings-the-palette-and-shortcuts-as-built).)*
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

## The engine bridge as built

The second step of v0.2 (2026-11-01) connected the shell to the engine; the IPC contract itself is in [ARCHITECTURE.md, "UI to backend"](ARCHITECTURE.md#1-ui-to-backend-tauri). Decisions it took for the UI:

- **The app owns the engine.** It opens the engine and its database in the background as the app starts, so the first frame never waits for it (#28). While the engine is starting, or while a `boltget` CLI owns the database, the store's status says so (`starting`, `locked`); the list's own wording for these states arrives with the list. The engine's error keys and the app's are in the string catalog, and `errorText()` falls back to a generic message for a key the catalog does not know.
- **Debug builds** use `BOLTGET_DATA_DIR` like the CLI, so development never touches real downloads, and expose the store and the generated commands to the devtools console as `__boltget`. There is no debug view.
- **The store** (`apps/desktop/src/lib/engine`) keeps one `Download` object per download with a `$state` field per value, and the list of ids separately. A progress tick updates only the fields that changed, so only those rows re-render; the id list changes only when downloads are added or removed. This is the data side of the budget in #33.
- **Optimistic actions** (#33): pause, resume, remove and speed limits change the row at once and roll back if the backend refuses, recording the error for a quiet notice. A new download's row comes back from `add_download` directly, before the engine's first event.
- **Speed is averaged in the backend** over 3 s (#27), so every view (rows, tray, taskbar) shows the same number; the UI still decides how often to repaint digits.

## The list and the details pane as built

The third step of v0.2 (2026-11-01) built the main list (#3, #4) and the details pane (#10 to #12). Decisions it took where this brief was open, and where it deviates:

**The list**
- **Rows** are as specified: 40 px comfortable (name with the state or the primary action at the right; a 4 px square progress bar and one muted status line below) and 32 px compact (`Ctrl+Shift+D`, one line, the fill alone as a 2 px underline with no track, so a column of rows does not read as ruled lines). The fill is the mark's gradient while a download runs and neutral grey once it stops.
- **Units are binary everywhere**, speeds included (`48.2 MiB/s`, where #3's example said `MB/s`), so a row never mixes two unit systems. Numbers that change live keep their trailing zeros (`0.40 / 5.97 GiB`) so their width does not jump; settled sizes drop them (`7 MiB`). Digits repaint on one shared 2 Hz beat.
- **Groups.** Failed, corrupt and expired-link downloads sit at the top of "Queued and paused" rather than in a group of their own. Section headers do not stick. Every view is grouped the same way; a sort other than Grouped gives one flat list (Name, Size, Date added).
- **"Rows never move on their own"** is implemented as: sections are held while the pointer moves over the list, it scrolls, or a key is pressed in it, and for 2.5 s after the last of these (0.3 s after the pointer leaves). A pointer resting on the list without moving does not count as using it, so history still settles while the user looks elsewhere. Adding or removing a download does move rows: the user did it.
- **One header line above the list** with the view's title and count, the sort menu and the pane toggle. It holds view options, not actions on downloads, so it is not the toolbar #1 rules out.
- **Selection and keyboard** follow Explorer: click, `Ctrl`+click, `Shift`+click, arrows, `Home`/`End`/`PageUp`/`PageDown` with `Shift` to extend and `Ctrl` to move without selecting, `Ctrl+A`. The list is one `listbox` with `aria-activedescendant`, so focus stays on the list and rows can be virtualised; the row's two buttons (primary action, more) are a pointer shortcut and are created only for the hovered or keyboard row. The keyboard reaches the same actions through the shortcuts and the `Menu` key or `Shift+F10`.
- **Remove is undoable** (DESIGN-PRINCIPLES #6) without engine support: the row hides for 6 s with "Removed … Undo" and a running download pauses at once; the removal is made when the notice ends. Quitting within those 6 s keeps the download. Deleting the file asks first, in an in-window dialog (the only confirmation in the app; Cancel has focus).
- **Context menus are native** (#29). Outside Tauri (the UI alone in a browser) there are none, and the sort button cycles instead.
- **Copy link** copies the link as the engine keeps it for display, without its query string or credentials.
- **Engine states** take the list's place calmly: "Opening your downloads…" fades in only if opening takes over 400 ms; a CLI holding the database shows a lock, one sentence and the folder in Mono; a failure shows the catalog message with the diagnostic in Mono below. A view or search with nothing in it says so in one line.
- **Adding** was a stand-in until the add sheet existed: pasted and dropped links started at once. Step 4 replaced it with the sheet (see [The add sheet as built](#the-add-sheet-as-built)).
- **Smoothness (#33)** was measured with 500 simulated downloads, 20 of them running, in Chromium at 1280 × 800: no long tasks; flinging through the whole list at about 3,600 px/s with the CPU throttled 4× (a mid-range laptop) holds 57 fps with no frame over 33 ms, and progress ticks alone stay at the display's rate. A progress tick re-renders only the rows on screen whose numbers changed.

**The details pane**
- **It is 360 px wide** and appears when it is on (`Ctrl+I`, or the header's toggle) and something is selected; with several downloads selected it shows their count and total size. Under 1000 px of window width it floats over the list with the floating elevation instead of squeezing it.
- **Top:** name (two lines at most), host and size, the primary action as a button (solid emerald only for Open), Show in folder and the same menu as the row. Then the hero card's stats: speed, received, time left and connections while running; size and when it finished once complete; received and what is left while stopped.
- **The file map** is a canvas of 8 rows of whole-pixel squares (about 9 px with 2 px gaps), each square a slice of the file along one emerald gradient (tokens `--color-map-*`). Arriving squares ease in with a brief light; the square a connection is writing is deep ink (mint on dark) with a halo; a finished download settles over 0.7 s into one calm field at about 45% opacity. Connections are numbered from 1, though the engine counts from 0. A stopped download whose map was not streamed this session shows its received share filled in file order. A file of unknown size shows one line instead of a map. Hovering a square shows its byte range and its connection. Reduced motion shows every square settled.
- **Below the map:** the speed over the last minute (2 Hz samples, a line on a faint area) and the connections at work, each with its part's progress. The engine does not report events such as "took half of #3" yet, so the list has no events.
- **File:** saved or saving to, the link, a refreshed link if there is one, the checksum (none given, checked at the end, verified, or does not match; its digest and, when the server sent it, the header) and whether the download can continue after a stop, each with a copy button.
- **Speed limit** for that download alone, in MiB/s, applied on `Enter` or leaving the field; empty is no limit.
- **Problems** (failed, corrupt, expired link) appear above the stats as one sentence from the catalog with a status glyph; an expired link gets the "Refresh link" field right there. The engine's English diagnostic, the only place an HTTP code appears, is folded under "Technical details". There is no Log section yet (the engine keeps no per-download log) and no connections stepper (the engine cannot change a running download's connection count); both wait for engine support.

**Development aids**
- The UI alone in a browser (`pnpm --filter @boltget/desktop dev`) runs on a simulated engine in development builds: `?mock=500&active=20` seeds downloads, `?engine=starting|locked|failed` holds a state, `?theme=light|dark` forces a theme. It is never part of a release build.

## The add sheet as built

The fourth step of v0.2 (2026-11-04) built the add sheet (#6 to #8) and polished the details pane. Decisions it took where this brief was open, and where it deviates:

- **Every way in opens the sheet; nothing starts on its own.** The "Add download" button and `Ctrl+N` open it with the links on the clipboard, if any (DESIGN-PRINCIPLES #6); `Ctrl+V` outside a field and a dropped link open it with those links. Links that arrive while it is open are added to its field. This settles the open question "auto-start on paste" for v0.2: the sheet, with Download as the one decision (`Enter`).
- **It is a modal sheet inside the window,** 560 px wide, 72 px from the top, on the floating elevation over a scrim: a native `<dialog>`, so focus stays in it and `Esc` (or a click on the scrim) closes it. It enters in 220 ms (fade and 8 px drop, `--ease-enter`) and leaves in 150 ms; reduced motion shows and hides it at once.
- **The probe** (`Engine::probe` tries a transient failure only twice, so an unreachable server is reported in about 30 s rather than minutes) runs as the link is typed (after a 350 ms pause) or at once when it is pasted, dropped or prefilled. One probe is in flight for a single link (three for a list); a link that left the field before its turn is skipped, and answers that arrive after the sheet closed are ignored, since the backend cannot abort a probe. Each link is probed once per sheet. "Looking at the link…" appears only after 150 ms.
- **What the preview shows:** the type icon, the resolved name as an editable field that looks like text until hovered, then one muted line: size, host, "Can resume" or "No resume", and the server's checksum ("SHA-256 from the server") when the probe saw one. The engine's probe now reports that digest (`ProbeResult::server_digest`).
- **One notice at most,** in this order: web page, not enough space on the drive, downloaded before ("You downloaded this on Oct 12." with Open, and the button becomes "Download again"), already in the list (with "Show it", which selects the row), and "saved as `name (1).ext`" when the name is taken. A failed probe replaces the preview with one sentence: unreachable (with Try again), sign-in needed, or the catalog's message. Downloading stays possible in every case: the sheet warns, it does not block. #7's "single connection and no resume" is the "No resume" in the meta line, not a notice.
- **Duplicates** compare links as the list keeps them (origin and path; query, fragment and credentials are not stored). A name already taken on disk, by a partial file or by another download is renamed with ` (1)` (docs/ENGINE.md, "Filename resolution"), never overwritten; the sheet shows the final name before starting (`Engine::free_path`).
- **The folder** was the last one used, else the Downloads folder; step 6 replaced that with the category's folder, else the default download folder ([Settings, the palette and shortcuts as built](#settings-the-palette-and-shortcuts-as-built)). A button shows it with the folder name in full and the path above it clipped from the start; it opens a native menu with the last three folders, the Downloads folder and "Choose folder…" (the system folder picker, called from Rust; the page gets no dialog or file system commands). The last three folders are stored in the engine's database as an app setting (`app.recent_folders`). A folder that does not exist yet is created when the download starts, and the sheet says so.
- **"More options"** holds what the engine honours today: connections (empty is Auto, or 1 to 32), a speed limit in MiB/s, a checksum (`sha256:…`, `md5=…` or bare hex, checked like the engine's parser) and "Add paused" (`Engine::add_paused`: recorded paused, started with Resume). Queue, start later at a time, sign-in, headers and referrer wait for their engine features. The disclosure remembers whether it was open. A bad value says so under the options and keeps the sheet open.
- **Several links** (one per line, or any text with links in it) become a checklist: each with its probed name and size, "Web page" or a warning glyph, a select-all checkbox with the count, one folder and one set of options for all, and "Download 3 files". The checksum field is hidden there, since one digest cannot fit several files.
- **After adding,** the sheet closes and the new rows are selected; a screen reader hears "Added …". There is no toast, since the user just did it (#16).

**Details pane polish**
- Paths and links wrap only after a folder separator (and a link's host after its dots, never right after `https://`), so a name is never split; each keeps its copy button and shows the full value as a tooltip.
- The speed chart's time axis covers what there is (at least 10 s) until a minute has passed, so a fresh download draws from the left edge; its label says "Speed since it started" until then, the newest sample has a dot, and the peak speed is shown beside the label.

## The tray, the taskbar and notifications as built

The fifth step of v0.2 (2026-11-04) built #13 to #16 and the app's life around its window. Decisions it took where this brief was open, and where it deviates:

**The tray (#13)**
- **The icon** is the app icon (the dark tile), so it reads on light and dark taskbars. It is drawn in Rust from the brand geometry (`apps/desktop/src-tauri/src/mark.rs`): the pixel-snapped 16 px design at 16, 20 and 32 px, the 24 px design at 24 and 48 px, at the size Windows asks for small icons.
- **The motion** is the brand's (BRAND.md §6): the arrow's rows light up top to bottom on a 2.6 s loop with a 0.12 s stagger, shown at 10 frames a second. It runs only while bytes are flowing (some download running at a speed above zero); it stops on the still mark when everything is paused, stalled or done. There is no setting of its own: it never moves when Windows' "Animation effects" are off, the system's reduced-motion switch.
- **Left click** opens or focuses the window. **The menu** is native: Open Boltget, Add from clipboard (opens the window and the add sheet with the clipboard's links; nothing starts on its own), Pause all, Resume all (paused downloads; failed ones wait for the user, as in the window), Speed limit ▸ (No limit, 1, 5, 10, 25 and 50 MiB/s, checked to match the global limit, which the window or the CLI may change), Open downloads folder, Quit Boltget.
- **The tooltip** is `3 active · 48.2 MiB/s · about 2 min left` (binary units, as everywhere), only `3 active` while nothing flows yet, and `Boltget` when idle. Its numbers change at most once a second.

**The taskbar button (#14)**
- It shows the **batch**: every download that ran since the user last looked. Green with the batch's progress while anything runs (red if something in it needs attention meanwhile), indeterminate when a size is unknown; when nothing runs, yellow if something in it is paused, red if something failed, needs a new link or is damaged, and nothing once everything in it completed. The batch is cleared when nothing runs and the window gains focus, so a failure stays red until the user comes back, and old paused downloads never colour the button. Tauri's `set_progress_bar` (ITaskbarList3) is enough; no Windows code of our own. The jump list waits for a later step.

**Notifications (#16)**
- Native toasts, built with `tauri-winrt-notification` (Tauri's own notification plugin cannot report clicks on Windows). *Completed*: the file name, then `420 MiB in 7.9 s` when this session saw the whole download (only the size when it continued from a checkpoint, where the time of this run alone would mislead), with Open and Show in folder. *Needs attention* (failed, link expired, damaged): the name and the catalog's sentence for the reason, with Fix…. Clicking a toast brings the window forward with the download selected and the details pane open; Fix… on an expired link opens its "Refresh link" field.
- **No spam.** Notices that arrive within 1.5 s of each other are one moment. Three or more completions within a minute become one summary (`5 downloads finished`, two names and "and 3 more"), and further completions in that minute add no toast; three or more problems at once become one summary too. Nothing is shown while the window is visible and focused, since the list says it already, and nothing for what the user did (a pause is not a notice). Windows holds toasts back during Focus sessions and Do not disturb by itself.
- There was no setting to turn them off in this step; Settings added one (see [Settings, the palette and shortcuts as built](#settings-the-palette-and-shortcuts-as-built)). Windows' per-app notification switch works too.
- **Development builds** show toasts as "Windows PowerShell": Windows shows toasts only for an app id it knows, and only the installer's Start menu shortcut registers Boltget's (`app.boltget.desktop`). Clicking an old toast in the notification centre after it left the screen does not reach the app (that needs a COM activator, later).

**The app around its window (#15)**
- **Close goes to the tray** (unless Settings says otherwise, from step 6); the first time a toast says so ("Boltget is still running. Downloads go on in the background. Quit from the tray icon."), recorded in the database so it is said once. Quit is in the tray's menu. Destroying the WebView after some minutes hidden is not done yet; the hidden page gets no batches, as before.
- **One instance.** A second `Boltget.exe` passes its links to the first, which opens the add sheet with them; without links it brings the window forward. `Boltget.exe <url>` on a first start opens the sheet once the engine is ready. `--background` starts in the tray, the window hidden.
- **The end of the session** checkpoints everything and continues the downloads that ran on the next start ([ARCHITECTURE.md, Recovery on start](ARCHITECTURE.md#recovery-on-start)); quitting from the tray leaves them paused.
- **The CLI** reaches the app over its named pipe ([ARCHITECTURE.md, CLI to app](ARCHITECTURE.md#3-cli-to-app-named-pipe)). A `boltget get` in a terminal adds the download directly, without the sheet: the user already decided. (#6's "CLI handoff" that opens the sheet is the shell's `Boltget.exe <url>`.)
- **Strings** the tray and toasts show come from the page's catalog (`packages/ui/src/messages/en.json`), compiled into the Rust side (`catalog.rs`) with the same plural and unit rules as `format.ts`.

## Settings, the palette and shortcuts as built

The sixth step of v0.2 (2026-11-06) built Settings (#17, #18), categories, clipboard monitoring (#9), the system proxy, the command palette and the shortcuts (#20). Decisions it took where this brief was open, and where it deviates:

**Settings (#17)**
- **A screen of the main window, not a window of its own** (a deviation from #17). `Ctrl+,`, the sidebar's Settings or the palette replace the sidebar and the list with Settings: its sections where the sidebar was, "Back to downloads" at the top, and the content sheet with one readable column (760 px at most). Esc outside a field, `Ctrl+,` again or Back return to the downloads. Reasons: the Windows 11 inbox apps (Settings, Terminal, Photos) show settings as a page; a second WebView2 would cost memory and a second copy of the theme, title bar and snap layout plumbing for a screen opened rarely; and the add sheet, the palette and notices keep working above it.
- **Sections:** General (theme, start with Windows, keep running when the window closes, notifications, offer copied links), Downloads (download folder, categories), Connections and speed (connections per download, speed limit), Network (proxy), Shortcuts and About (version, licence, "No ads, no tracking, no bundled offers", #19). Browser arrives with v0.3; Advanced was not needed: every engine value the app exposes has a plain row.
- **Search is the title bar's field,** which says "Search settings" while Settings shows (`Ctrl+F` or `/`). It lists the rows whose label, description or synonyms contain every typed word, under their section titles; ↓ or Enter moves into the first one. The same rows feed the palette.
- **Rows** are cards in the Windows 11 manner: label, one sentence, the control on the right (a switch, a drop-down that is the native `<select>`, a text field, a folder button), and anything more below inside the card. New primitives in `packages/ui`: `Switch` (`role="switch"`), `Select`, `TextField`.
- **Changes apply at once,** with no Save button: switches and choices when they change, typed values on Enter or when the field is left. The page checks a typed value first and says what is wrong under the field in red (`aria-invalid`, `aria-describedby`, `role="alert"`); Esc in a field puts the stored value back. The backend checks again and refuses anything else with the same words. A failed switch flips back.
- **Applies where possible:** the theme, notifications, close to tray, clipboard monitoring and the global speed limit at once; the proxy and the connection cap for downloads that start or resume from then on (a running download keeps its connections; the description says so). Start with Windows writes the current user's Run key at once.
- **Folders** are chosen with the system's folder picker (no typed paths, so no path to validate); a set folder has a clear button that goes back to the default.
- **The theme setting** (Use Windows setting, Light, Dark) is stored with the others and cached in the page's `localStorage`, so the first frame of the next start already has it. `BOLTGET_DEV_THEME` and `Ctrl+Shift+L` still win in debug builds.
- **Not done here:** #18's "the first window says where downloads go" line, and offering clipboard monitoring "the first time a copied link is seen", which would mean reading the clipboard while the setting is off; the opt-in is the switch in Settings (and the palette).

**Categories and where downloads go (#2, #6)**
- **Six built-in categories** (Video, Music, Archives, Programs, Documents, Other) with editable file types (typed as `mp4 mkv`, with or without dots, commas or `*.`; one extension belongs to one category, and the error names the other one) and an optional folder each. Other has no types: it is everything else. Adding or renaming categories is not in v0.2.
- **The add sheet picks the folder** (a deviation from step 4's "last folder used"): the category's folder for the probed file's type, else the default download folder from Settings, else Windows' Downloads folder; a line under the folder says "The Video folder, from Settings." when a category chose it. The folder menu still offers the last three folders, and a folder the user picks wins for that sheet. A list of links goes to the category folder its files share, else the default. The tray's and the palette's "Open downloads folder" open the default folder.

**Clipboard monitoring (#9)**
- Off by default; "Offer copied links" in General. While it is on, Windows' clipboard change notification wakes a listener (no polling); off, nothing reads the clipboard ([ARCHITECTURE.md, Clipboard monitoring](ARCHITECTURE.md#clipboard-monitoring)).
- **What is offered:** clipboard text that is nothing but http(s) links, where a link names a file of a known category (by its name, or a `filename` in its query, as signed storage links carry it). Prose with a link in it, web pages and other schemes are not offered; a link is offered once per session; copies made in Boltget and content marked private by password managers are ignored.
- **The offer** is a quiet in-app notice, "Download clip.mkv from example.com?" with "Download…", for 12 s; while the window is not in front a notification says the same (when notifications are on). Taking it opens the add sheet with the links; nothing is ever added on its own.

**Notifications and closing (#15, #16)**
- "Notifications" turns off every toast (completions, problems, the close-to-tray explanation and copied-link offers); Windows' per-app switch still works.
- "Keep running when the window closes" is on by default; off, the close button quits (downloads checkpoint and stay paused, as with the tray's Quit).

**The command palette (#20)**
- `Ctrl+K` opens it over either screen (it is a modal `<dialog>`, 620 px wide, 12 % from the top, on the floating elevation, 120 ms in, nothing under reduced motion); `Ctrl+K` again or Esc closes it. Search moved to `Ctrl+F` and `/`.
- **Before anything is typed** it lists, in groups: the selected downloads' actions (only those their states allow, with the file name), Actions (add, pause all, resume all, search, details, row density, open downloads folder, open settings, the speed limit presets 1, 5, 10, 25 and 50 MiB/s and "Remove the speed limit", the two other themes, the switches worded as what they will do, "Quit Boltget"), and Go to (the views and categories). Typing ranks everything, settings rows and every download (newest first) included, in one list.
- **Matching:** every typed word must match the label or the entry's synonyms ("throttle" finds the speed limits), in any order; a word's letters as a run at a word start score highest, then a run anywhere, then letters in order ("pa" finds "Pause all"); synonyms score a little lower; ties prefer the shorter label. Results show their shortcut as keycaps.
- **Keys:** ↑ ↓ (wrapping), PageUp/PageDown, Enter runs, Tab shows the result in context (a setting's row, focused and briefly outlined; a download selected in "All" with its details), Esc closes. The field is a `combobox` over a `listbox` with `aria-activedescendant`, and the result count is announced politely.

**Shortcuts**
- One list (`apps/desktop/src/lib/shortcuts.ts`) feeds the Shortcuts section and the palette's keycaps: `Ctrl+K` palette, `Ctrl+N` add, `Ctrl+V` add a copied link, `Ctrl+F` or `/` search, `Ctrl+,` settings, `Ctrl+Shift+P`/`Ctrl+Shift+R` pause/resume all, `Space` pause or resume the selection, `Enter` open, `Ctrl+Enter` show in folder, `Ctrl+C` copy the link, `Delete` remove (undoable), `Shift+Delete` remove and delete the file, `Ctrl+A` select all, `Shift+F10` or the Menu key more actions, `Ctrl+I` details, `Ctrl+Shift+D` compact rows, arrow keys, `Home`/`End`/`PageUp`/`PageDown` to move. Remapping waits for a later release.

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

- **Auto-start on paste.** Should `Ctrl+V` in the window start the download at once, with an undoable preview row (Downie and browsers), or open the sheet with Download focused (one `Enter`)? *v0.2 ships the sheet* ([The add sheet as built](#the-add-sheet-as-built)); auto-start stays an option to test with users.
- **A table layout for IDM users,** or one row model only. Rows ship in v0.2; a table is added later if users ask.
- **Segments in the row.** Should the progress bar of an active row hint at gaps (a 1 px mini map), or should segments appear only in the details pane? *For now only in the pane;* rows keep one plain bar.
