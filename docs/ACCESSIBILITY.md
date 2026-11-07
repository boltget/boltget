# Accessibility of the desktop app

How the v0.2 app meets the accessibility part of [DESIGN-PRINCIPLES.md](DESIGN-PRINCIPLES.md#5-accessibility) (WCAG 2.2 AA), screen by screen, how that was checked, and what is still open. The design of each screen is in [APP-DESIGN.md](APP-DESIGN.md).

## How it was checked (2026-11-07, v0.2 step 8)

- **The real window.** The release build (`Boltget.exe`, WebView2 156) on Windows 11 25H2 at 100% scaling, with 500 downloads in the list. Names, roles and states were read from the **Windows UI Automation tree**, the same tree Narrator and NVDA read, with a PowerShell script over `UIAutomationClient` (control view; Chromium builds the tree on the first UIA request, so the dump is taken twice).
- **Keyboard and focus** through the Chrome DevTools Protocol (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`): each screen opened with its shortcut, where focus lands, the Tab order (12 presses), and where focus returns when it closes.
- **Reduced motion, forced colours and zoom** with the protocol's media emulation (`prefers-reduced-motion: reduce`, `forced-colors: active`) and device metrics at 150% and 200% of the default window (half the CSS pixels at twice the density, which is what Windows text scaling and browser zoom do to the page).
- **Contrast** computed from the generated tokens, in both themes, by `packages/ui/src/contrast.test.ts`, which runs in CI (`pnpm --filter @boltget/ui test`).
- **Not done:** listening with Narrator or NVDA. The checks above cover what they read, but not how they phrase it; a pass with both screen readers is a v0.5 exit criterion ([ROADMAP.md](ROADMAP.md#v05-scheduler-queues-and-polish)).

## Results per screen

| Screen | Keyboard and focus | Names, roles, states (UIA) | Live regions | Motion, high contrast, zoom |
|---|---|---|---|---|
| Title bar | Search, Add download in order; caption buttons are mouse-only (`tabindex=-1`), as in Windows | Edit "Search downloads", buttons named | – | Fine at 200% |
| Sidebar | Every view, category, queue and Settings is a button in Tab order; ring visible | `List` "Views"/"Categories"/"Queues", current view `aria-current`, counts read as "7 downloads" | – | At 200% it becomes the icon rail with tooltips |
| List | One Tab stop; arrows, Home/End, PageUp/PageDown, Shift/Ctrl as in Explorer; the active row has the emerald ring | `List` "Downloads" (multi-select listbox with `aria-activedescendant`); each row a `ListItem` named by file and state ("ubuntu.iso 42%"), with its status line as description; *fixed:* the row's name included its hover buttons; progress bars were all named "Downloads" | Completions and failures are announced politely, never progress | Bars were invisible in forced colours (*fixed*); idle bars below 3:1 (*fixed*) |
| Details pane (`Ctrl+I`) | In Tab order after the list; the close, primary action, Show in folder, menu, copy buttons and the speed-limit field | `Group` "Details" (complementary), the file name `h2`; *fixed:* section titles were plain text, now `h3` headings; the toggle in the header changed its label with its pressed state, now a fixed "Details" toggle | No live region (numbers change twice a second; they are read on demand) | The file map is a canvas with a text label; settles at once under reduced motion |
| Add sheet (`Ctrl+N`, `Ctrl+V`) | Opens with focus in the link field; Tab cycles inside; Esc closes and focus returns to the list; *fixed:* Tab after the last control left the page for one stop | `Window` "Add download" (modal dialog); the head's × was a second "Cancel", now "Close"; More options exposes its expanded state | Probe status is a polite region; errors `role=alert` | No entrance motion under reduced motion; scrolls inside at 200% |
| Command palette (`Ctrl+K`) | Focus in the field; ↑↓ move, Enter runs, Tab shows the result in context (by design, it closes the palette) | `ComboBox` over a `List` with groups; each result a `ListItem` with its shortcut | Result count announced politely | No motion under reduced motion |
| Settings (`Ctrl+,`) | *Fixed:* focus was lost when Settings replaced the list; now on the current section. *Fixed:* Esc did nothing on a switch or a drop-down; now it returns to the downloads (it still restores a text field) | Sections `aria-current`; each row a group with its label and description; switches are `role=switch` with toggle state, drop-downs native | Field problems `role=alert` | Fine at 200% |
| Queue settings, Start later | Focus in the first field; Tab cycles inside; Esc cancels and focus returns | Modal dialogs named by their title; problems `role=alert` tied with `aria-describedby` | – | No motion under reduced motion |
| Delete file (Shift+Delete) | Cancel has focus; Tab cycles between Cancel and Delete file; Esc cancels | Modal dialog with title and body | – | – |
| Finish countdown | Cancel has focus, Enter or Esc cancels; Tab stays inside | `alertdialog` named by "Shutting down in 0:59" | The dialog itself is the alert; the time is not re-announced every second | *Fixed:* the emptying bar was invisible in forced colours. Checked by code review only: a release build really sleeps or shuts down |
| In-app notices (Undo, copied link) | The action button is reachable; notices do not take focus | `role=status` | Polite | Static under reduced motion |
| Tray, toasts, taskbar | Native: the tray menu and toasts are Windows' own and keyboard reachable as such (Win+B for the tray) | Toasts show as "Boltget" from the installed app (app id `app.boltget.desktop`) | Windows announces toasts | The tray icon is still when Windows' animation effects are off |

**Across screens** (also fixed in this step): Tab no longer leaves the window for an empty stop after the last control; there are no WebView2 permission prompts (the browser's "wants to see the clipboard" prompt appeared on the first `Ctrl+N` with a fresh profile and was not keyboard-friendly). Reduced motion: with `prefers-reduced-motion: reduce` no element in the window had a running animation or a transition longer than 10 ms, apart from colour and opacity fades.

## Contrast

Ratios from the generated tokens; every pair below is a test case in `packages/ui/src/contrast.test.ts` (text 4.5:1, control boundaries and state glyphs 3:1).

| Pair | Light | Dark |
|---|---|---|
| Text on content / chrome / raised | 19.6 / 18.2 / 19.6 | 18.8 / 17.6 / 16.3 |
| Muted text on content / chrome / raised | 7.7 / 7.2 / 7.7 | 7.7 / 7.2 / 6.6 |
| Subtle text on content / chrome / raised (*was 4.8 / 4.5 / 4.8 and 4.1 / 3.8 / 3.5*) | 5.4 / 5.0 / 5.4 (4.6 on a hovered chrome row) | 6.1 / 5.7 / 5.2 (4.7 on a hovered raised row) |
| Accent (links, ghost buttons) on content | 5.5 | 10.2 |
| Error text on content / raised | 4.8 / 4.8 | 7.1 / 6.1 |
| Text on the primary button | 5.5 | 7.9 |
| Focus ring on any surface | 5.1+ | 8.8+ |
| Field and checkbox edge (`control-stroke`, *new*: was a 1.4:1 hairline) on content / chrome / raised | 3.7 / 3.4 / 3.7 | 4.1 / 3.8 / 3.5 |
| Stopped progress fill on content (*was 2.6 and 2.5*) | 3.7 | 3.6 |

## Open

- A listening pass with Narrator and NVDA over every screen (v0.5).
- Long paths and links in the details pane are split into several UIA text runs where they may wrap; screen readers read them in order, but a reader that announces runs separately may pause between them.
- The window's minimum size (640 × 440) at 200% leaves a 320 × 220 CSS-pixel page: usable, with the sidebar as an icon rail and the list scrolling, but cramped. Reflow to 320 CSS pixels (WCAG 1.4.10) is not claimed for the app.
