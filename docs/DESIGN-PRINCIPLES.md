# Design principles

How BoltGet looks, moves, reads and behaves, in the desktop app, the extension and on boltget.app. Visual tokens (colors, type scale, spacing, radii, the icon grid) are defined in [brand/BRAND.md](../brand/BRAND.md); this document is about how to use them and the product decisions behind them.

The ethos in one line: **simple, but makes you say wow.** The wow comes from restraint, great typography and motion that explains the product, never from decoration.

## 1. Restraint

- **One primary action per view.** In the main window it is "Add download". In the add sheet it is "Download". Everything else is secondary or behind a disclosure.
- **Defaults that are right for 90% of people.** Settings exist for the other 10%, grouped and searchable, never in the first-run flow. Every new setting needs a reason it cannot be a good default.
- **No chrome for its own sake.** No toolbar of twenty icons, no status bar repeating what the list shows, no splash screen longer than the app needs to start.
- **Color is information.** The brand gradient is reserved for the BoltGet mark, the active-download indicator and the primary button. Status colors (paused, error, done) are used only on status, never decoratively.
- **Say less.** If a label needs a tooltip to be understood, rewrite the label.

## 2. Information density done right

A download manager is a list of things that change. Power users want many rows on screen; new users want to understand each row. Both are possible:

- **Rows show what matters at a glance:** file icon by type, name, a progress bar, size (done / total), speed, time remaining, state. Nothing else by default.
- **Numbers are tabular.** Use tabular (monospaced) figures for sizes, speeds and percentages so columns do not jitter as values change.
- **Stable layout under change.** Values update in place; rows never reflow, resize or reorder on their own while the user is looking at them. Sorting by "speed" does not re-sort live; it re-sorts when the user asks.
- **Human units, consistently.** Binary units with clear labels (`MiB`/`GiB` in technical contexts, configurable to `MB` decimal), speeds per second, rounded sensibly (`48.2 MB/s`, not `48.23412 MB/s`). Time remaining in words when long (`about 3 min`), precise when short (`12 s`).
- **Details on demand.** Selecting a download opens a details pane: the live segment map, connections, server, URL, checksum, log. This is where the depth lives.
- **Two densities.** Comfortable (default) and Compact (toggle, `Ctrl+Shift+D`), changing row height and padding only, never hiding columns silently.

## 3. Motion that explains

Motion is how BoltGet shows what the engine is doing. It is never ambient decoration.

**Signature motion:** the animated pixel arrow, rows lighting up top to bottom like chunks arriving (see [brand/BRAND.md](../brand/BRAND.md)). It appears in exactly these places:

- App launch / splash (once, short).
- Tray icon while at least one download is active. Static when idle.
- The website hero.
- Empty state of the main list (static or a single play, not a loop).

**Product motion rules:**

- **Explain state changes.** A segment splitting in two animates as one bar dividing; a completed download settles into the "done" state; a failed one gets a single, brief color emphasis (no shaking, no bouncing).
- **Durations:** design targets 120–200 ms for small UI transitions (hover, press, toggles), 200–320 ms for layout changes (sheets, panes). Nothing over 400 ms except the one-time splash.
- **Easing:** ease-out for entering, ease-in for leaving, no springy overshoot on data.
- **Progress is continuous, not jumpy.** Progress bars interpolate between updates (the UI receives updates a few times per second, see [ARCHITECTURE.md](ARCHITECTURE.md#1-ui-to-backend-tauri)); speed numbers are smoothed so they are readable.
- **No infinite loops** except where something is genuinely, continuously happening (an active download's indicator).
- **`prefers-reduced-motion`:** all non-essential motion is removed; state changes become instant cross-fades or no animation. The tray icon shows a static "active" variant instead of animating. This is tested, not assumed.
- **Performance:** animate only `transform` and `opacity`; nothing that forces layout on every frame. The segment map draws on a canvas or with transforms, not by resizing DOM elements 60 times a second.

## 4. Dark-first, with a real light theme

- Dark is the default because the brand is built on near-black `#0B0B0F`, and because a download manager often sits open next to other work.
- Light theme is a first-class theme, designed and reviewed, not an inverted dark theme. Gradient and status colors are tuned per theme for contrast.
- "System" follows the OS setting and is the default *setting*; when the OS gives no preference, dark wins.
- All colors come from tokens in `packages/ui` (generated from [brand/BRAND.md](../brand/BRAND.md)). No hard-coded colors in components.

## 5. Accessibility

Target: **WCAG 2.2 AA** for the app, extension UI and website.

- Contrast: text at least 4.5:1 (3:1 for large text and UI component boundaries) in both themes. The mint/emerald gradient is never the only carrier of text contrast.
- Every interactive element is reachable and operable by keyboard, with a visible focus ring (designed, not the browser default, and never removed).
- Screen readers: proper roles and names; the download list is a grid/list with row semantics; progress uses `role="progressbar"` with values; live regions announce completions and failures politely, not every progress tick.
- Status is never color-only: icons and text accompany color (paused, failed, done).
- Hit targets at least 24×24 px (WCAG 2.2), comfortable targets 32 px and up in the app.
- Respect OS settings: text scaling, high contrast mode on Windows (`forced-colors`), reduced motion.
- Accessibility is checked in CI on the website (axe) and in UI tests for the app where feasible, plus manual checks with Narrator and NVDA before releases.

## 6. Keyboard-first

Everything can be done without a mouse. Default shortcuts (all remappable later):

| Action | Shortcut |
|---|---|
| Add download (from clipboard if it holds a URL) | `Ctrl+N`, or `Ctrl+V` in the main window |
| Command palette | `Ctrl+K` |
| Search / filter list | `Ctrl+F` or `/` |
| Pause / resume selected | `Space` |
| Pause all / resume all | `Ctrl+Shift+P` / `Ctrl+Shift+R` |
| Open file | `Enter` |
| Show in folder | `Ctrl+Enter` |
| Remove from list (undoable) | `Delete` |
| Remove and delete file (confirms) | `Shift+Delete` |
| Details pane | `Ctrl+I` |
| Settings | `Ctrl+,` |
| Move through list | arrow keys, `Home`, `End`, `PageUp`, `PageDown` |

- The command palette exposes every action, including settings ("limit speed to 2 MB/s", "open downloads folder").
- Destructive actions are undoable where possible (remove from list shows an "Undo" toast for a few seconds) and confirmed only when not undoable (deleting files from disk).

## 7. Behaviour principles

- **Never surprise with data loss.** Deleting a file from disk is a separate, explicit action. Removing from the list never deletes the file.
- **Never steal focus.** Downloads captured from the browser appear in the app (and optionally a small toast) without bringing the window to front, unless the user chose "ask where to save".
- **Ask once, at the right moment.** Permissions (extension host access, sidecar install) are requested when the user first needs them, with one sentence of why.
- **Errors say what happened and what to do,** in plain words: "The link expired. Get a fresh link from the site and drop it here to resume (73% is kept)." Not "HTTP 403".
- **Notifications are earned:** one per completed download (configurable), one per failure that needs action. No promotional notifications, ever.
- **Respect the system:** native file dialogs, native notifications, OS accent color for focus rings where it fits the palette, standard window controls.

## Copy and strings

- **Language:** the product is English only. Localization may come later as a community effort (see [TECH-STACK.md, ADR-015](TECH-STACK.md#adr-015-english-only-product-with-a-central-string-catalog)), so:
  - **Every user-facing string lives in the central string catalog** and is referenced by key. No literals in components, no user-facing strings built in the engine (the engine returns keys and parameters).
  - No string concatenation to build sentences; use whole messages with named placeholders (`"{count} downloads paused"`), and plural-aware messages for counts.
  - Do not bake text into images or icons.
- **Tone:** calm, direct, competent. Like a good engineer explaining something to a friend. No hype inside the product ("Blazing fast!"), no cute errors, no exclamation marks except for genuinely good news.
- **Voice:** address the user as "you" sparingly; prefer describing the thing ("Download paused") over narrating the user ("You paused the download").
- **Buttons are verbs:** "Download", "Resume", "Refresh link", "Show in folder". Not "OK" when a verb exists.
- **Sentence case** everywhere (titles, buttons, menu items), not Title Case.
- **Numbers and units** follow the [density rules](#2-information-density-done-right).
- **Website copy** can be bolder than in-app copy (it is selling), but every claim on it must be true and, for speed claims, backed by the published benchmark.

## Website-specific

The website follows the same principles with its own constraints (near-zero JavaScript, performance budget). See [WEBSITE.md](WEBSITE.md).

## Review checklist for UI changes

Before a UI PR is merged (also in the PR template):

- [ ] Works and looks right in dark and light themes.
- [ ] Fully keyboard operable, with visible focus.
- [ ] Checked with reduced motion.
- [ ] No hard-coded colors, sizes or strings (tokens and the string catalog only).
- [ ] Screenshots or a short recording attached.
