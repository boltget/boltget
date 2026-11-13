# Website

boltget.app is the product's front door: it explains Boltget in seconds, proves the speed claim, gets the installer onto the user's machine, and hosts the user documentation.

- Code: `apps/web` (package `@boltget/web`): `pnpm install`, then `pnpm --filter @boltget/web dev` (or `build`, `preview`, `check`). CI: `.github/workflows/web.yml`. What is built so far, and the decisions made while building it, are under [Implementation status](#implementation-status).
- Stack: Astro (static output) with plain TypeScript demos, user docs with Astro Starlight in the same project, shared components and tokens from `packages/ui` (until `packages/ui` exists, the site imports `brand/tokens/tokens.css` and `brand/fonts` directly). Rationale: [TECH-STACK.md, ADR-010](TECH-STACK.md#adr-010-astro-and-starlight-for-the-website-and-docs).
- Language: English only, with all copy in the site's string catalog or content collections (no copy scattered through components), so a community translation stays possible later. See [DESIGN-PRINCIPLES.md, Copy and strings](DESIGN-PRINCIPLES.md#copy-and-strings).
- Live by: v1.0 (a minimal landing page can go up earlier; see [ROADMAP.md](ROADMAP.md)).

## Site map

| Path | Page | Purpose |
|---|---|---|
| `/` | Home | The pitch in one screen; the hero; three proof points; download button |
| `/download` | Download | OS-detected primary button, all assets, checksums, winget command, system requirements, extension store links |
| `/features` | Features | The capabilities, each with a small explanatory visual, linking into docs |
| `/compare/idm` | Redirect | Retired; redirects to `/benchmarks/` |
| `/benchmarks` | Benchmarks | Methodology, dated results per version, raw data links ([ENGINE.md](ENGINE.md#benchmarks-plan-vs-idm)) |
| `/docs/...` | Documentation (Starlight) | Getting started, browser extension setup, video downloads, CLI reference, troubleshooting, FAQ |
| `/changelog` | Changelog | Rendered from `CHANGELOG.md` at build time |
| `/privacy` | Privacy | What the app, extension and site collect (nothing, by default), in plain words |
| `/blog` | Blog | Later (post-1.0): engineering deep dives, release notes |
| `/update/{channel}/latest.json` | Update manifests | Static files consumed by the app's updater ([RELEASE.md](RELEASE.md#auto-update)); not linked from pages |

The repository's design documents (this `docs/` folder) are for contributors. The website's docs are for users. They link to each other but are not the same content.

## Home page

Home was redesigned in October 2026 (the owner chose **direction A ("Daylight")** from the prototypes at `/lab/a` and `/lab/b`, see [Design lab](#design-lab)) and again in November 2026, after cap.so, as a framed light page around the real app. This section keeps the research and the October plan; what is built now, and the decisions taken while building it, are in [Home as built](#home-as-built) and [Character](#character). The first Home (a centred headline over the giant pixel arrow) is described under [Superseded: first hero concept](#superseded-first-hero-concept).

### Research notes

What makes the reference sites feel premium, in one line each (studied October 2026):

- **contravise.co** (the owner's reference): a looping atmospheric video field (deep blue to cyan) under a dark shade that fades to white; a 60 px Manrope headline at −0.05 em tracking, two lines, left; one sentence and two underlined text links on the right, no buttons; a large white card (24 px radius, soft shadow) overlaps the bottom of the field and plays the product live when it scrolls into view: a question types itself in large type, the answer appears, the cited clause highlights in the document beside it. Below: white sections, 40–56 px headings, prices written as a sentence with underlined amounts, a quiet FAQ.
- **linear.app**: near-black, headline and product UI fade and un-blur in sequence; the product screenshot is the hero, not an illustration; tight, small UI type around one big statement.
- **raycast.com**: one centred line of huge type, almost nothing else above the fold; the product window appears as the scroll reward.
- **vercel.com**: a single geometric object lit in an otherwise black field, hairline grid lines, a calm two-column hero.
- **family.co**: light, soft, playful motion with small floating objects; proves a light page can feel as premium as a dark one.

Shared traits: very few words; one centerpiece that is the real product (or a faithful simulation of it); type that is large, tight and calm rather than loud; motion that explains rather than decorates; long quiet whitespace; text links over button stacks.

### Concept: the page is a download

A download manager's craft is invisible: bytes arrive, a file appears. Boltget's mark already draws the idea (each pixel is a chunk). The redesign makes the whole page behave like a download: **the atmosphere is made of chunks, the centerpiece is a file arriving piece by piece, and every number is either measured or labelled as simulated.** No other download manager site looks like this, because none of them shows the thing actually happening.

Two directions were prototyped:

- **A, Daylight.** Light and atmospheric. A slow emerald-to-teal aurora fills the top of the page and, towards the bottom, breaks into square chunks with even gaps that thin out into white. A large white app card overlaps it and plays a simulated preview of the v0.2 desktop app (link typed, file probed, connections coming online, file map filling, checksum verified).
- **B, Block map.** Dark and bold. The whole hero is the map of a file: thousands of faint squares, each a slice of the file, light up as connections write them, in bright moving edges that settle to a faint emerald. In front, a terminal runs `boltget get` in the CLI's real output format, synchronised with the map behind it.

**Recommendation: A as the site's system, with B's block map as the second scene.** A is closest to what the owner asked for (light, airy, a product card as the centerpiece) and scales to every other page; B's map is the most distinctive single idea and is honest today (it shows the CLI that exists). The full build uses A's hero; the block map returns below the fold as the "how it works" scene, and becomes the dark docs/benchmarks hero accent. The owner decides; the plan below assumes this recommendation.

### Art direction

**Color.** *November 2026: monochrome, after the brand (see [brand/BRAND.md](../brand/BRAND.md), Palette and Atmosphere).* Everything is ink `#0A0A0B`, white, paper `#F5F5F3` and the graphite scale (`--graphite-50` to `-950`), used through the brand tokens (`brand/tokens/tokens.css`) rather than hex. The atmosphere is the "pearl sky": a grey cloud sky washed toward paper (`--atmosphere-haze`, `-pearl`, `-mist`, `-paper`, white). Text on it is ink; the primary call to action is an ink pill, the secondary a white pill on a hairline; links, the current item and focus rings are ink. Progress bars and file maps run along the graphite ramp toward ink (`--gradient-progress-light`), the square being written in ink. Colour only states something: paused amber (`--paused`) and error red (`--error`). No green, teal, blue or violet anywhere, with one exception: the platform logos in the "Available on" row (`PlatformLogo.astro`) keep their own colours (Windows blue, Tux in black, white and yellow-orange; Apple in graphite to ink, as Apple has no brand colour), at the owner's request, so the platforms read at a glance; the ones not yet available are grey and faded until hovered. Dark pages (the docs dark theme) use ink with white as the accent. Charts: Boltget in ink, the other download manager in graphite-300. The SmartScreen illustration on `/download` is graphite, not Windows blue. Finished rows in the hero app show an ink tile with a pixel check. *(Earlier plans used an emerald-to-teal atmosphere and emerald accents, then a sky-blue tile; both are retired.)*

**Type.** (October plan; the site now uses three families, see [Character](#character).) Geist 400/600 and Geist Mono 400 only (three weights, the budget). Scale: display `clamp(3.4rem, 6.6vw, 6.25rem)` / 0.95 / −0.055 em; statement `clamp(1.75rem, 3.6vw, 3.25rem)` / 1.12 / −0.035 em, weight 600; figure (measured numbers) `clamp(2rem, 3.4vw, 3rem)` / 1 / −0.045 em, or Geist Mono at up to 9.5 rem for B; body 17–20 px / 1.5; UI inside the demo 12–16 px; kicker label mono uppercase 13 px, +0.12 em, ink (one per section). Headlines never exceed two lines on desktop; `text-wrap: balance` for headings, `pretty` for paragraphs.

**Grid and spacing.** 84 rem max width, gutter `clamp(1rem, 4.5vw, 3rem)`. Hero: headline left (≈ 55%), sentence and links right, bottom-aligned. Sections are separated by large vertical rhythm (`clamp(6rem, 14vw, 11rem)`) rather than rules or cards. Card radius 20 px for the demo window only; UI chips are pills; everything pixel-related (map squares, field chunks) is square, never rounded.

**Motion principles.** Motion explains a mechanism or it does not ship. (1) Entrance: headline, aside and card rise 14 px and un-blur over 1.1 s with a 0.12 s stagger (`cubic-bezier(0.22, 1, 0.36, 1)`), once. (2) The demo is the only looping motion above the fold; it pauses off screen and in hidden tabs. (3) The atmosphere moves slowly enough to read as weather, not animation (noise drift of ≈ 0.03 units/s). (4) Chunks arrive linearly, like the brand motion; no bounce or overshoot. (5) `prefers-reduced-motion`: no loop anywhere; the demo shows its finished state, the field one still frame. (6) No scroll-jacking or smooth-scroll libraries.

### Hero and the live demo (direction A)

1. **Field.** A WebGL fragment shader (≈ 3 KB, no library) draws the aurora: domain-warped fbm noise plus one slow luminous "curtain" band, mapped through the atmosphere ramp. Below 50% of its height, the field is quantised into 13 px cells (10 px on phones) with 2 px gaps; cells drop out by a per-cell hash as the field approaches white, with a slow per-cell wobble so a few chunks come and go. Behind the card, this is the visible "chunks arriving" edge. Fallback without WebGL or JS: a CSS gradient with the same stops.
2. **Copy.** Header with the on-dark lockup and four quiet links. Headline *"Every byte, at full speed."*; sentence *"Splits every file across many connections and keeps each one busy until the last byte."*; two underlined links, *Try the CLI preview* and *See the benchmark*. No buttons in the hero.
3. **The card.** A white window (title bar with the 16 px icon, "Boltget", a **"Preview · v0.2 app"** chip, Windows caption buttons), sidebar (All downloads / Active / Completed / Scheduled with counts) and main pane. Timeline of one loop (≈ 16 s): empty state *"Paste a link or drop one here."* with a blinking caret → the link types itself (30 ms/char) → the Download button presses → "Connecting to cdn.example.com" → the job appears: file name, `420 MiB · cdn.example.com · resumable`, big tabular stats (speed, received, time left, connections), the **file map** (6 × 56 squares; each square a slice of the file, coloured by the connection that wrote it, the square each connection is writing in ink), a throughput graph drawing itself, and the connection list where rows appear as the controller adds them and show "took half of #3" when a connection steals work → **Done. 420 MiB in 7.9 s.** with "SHA-256 verified", the Completed count ticks from 11 to 12 → hold 4 s, fade, loop. "Earlier today" rows under the job keep the window from ever looking empty.
4. **Simulation.** `apps/web/src/lab/sim.ts`: seeded (mulberry32), fixed 1/120 s steps, start 4 connections, add one every 0.6 s while the last addition raised throughput by more than 6%, a wobbling 58 MiB/s link cap that makes more connections stop paying off (the demo settles at 7), work stealing on finish. It mirrors [ENGINE.md](ENGINE.md)'s behaviour, not its numbers.
5. **Honesty.** The chip says it is a preview of v0.2; the caption under the card says *"Simulated preview of the desktop app planned for v0.2. The numbers are illustrative."* with a link to the measured results. The figure's accessible name describes the simulation; the live numbers are `aria-hidden`. Until the app exists, the CLI is the primary call to action. When v0.2 ships, the card is replaced by a recording-faithful version driven by the real UI components from `packages/ui`.

### Home sections after the hero

Superseded by [Home as built](#home-as-built). The plan was a statement paragraph, three proof figures, a terminal vignette, a feature list, a CLI block and a closing line; the owner judged a first build of it "just text". The rule that replaced it: **every section shows, not tells**: a short headline, at most one line of copy, and a visual that does the explaining.

### Home as built

Redesigned in November 2026 after the owner rejected three static comps and asked for something in the spirit of cap.so, in Boltget's own style. This replaced the first build of direction A (the WebGL aurora hero with the simulated app card, the five numbered scenes and the night band; all in the git history). Components are in `apps/web/src/components/home/`, copy in `src/i18n/en.json` under `home.*`.

1. **Hero** (`pages/index.astro`). Its own rounded panel (18 px radius, fill `--atmosphere-mist`) inside the frame, with the pearl sky behind the copy: a grey cloud sky, no hue. The image is `src/assets/sky.avif` and `sky.webp`, made from the brand's source art (`brand/source/sky-master.webp`, see BRAND.md); `apps/web/scripts/sky.mjs` is the older generator of the blue "Daylight" sky and must not be rerun over it. Over the image a paper wash (`.panel::after`: a soft paper glow behind the copy and a vertical gradient toward paper at the foot) keeps the ink copy on light and melts the sky into the page. The layer drifts 5% sideways over 140 s (not under reduced motion). Centred: the headline *Every byte,* / *at full speed.* (the second line in Source Serif 4 italic), one serif sentence, an ink pill "Download for Windows" with the Windows mark, a white pill "See how it works" (to the Speed section), a muted "Windows 10 and 11", and a small "Available on" row of platform logos (`PlatformRow.astro`, `PlatformLogo.astro`, the colour exception above): the Windows 11 logo in its blue, linking to `/download`; macOS (the Apple logo, graphite to ink) and Linux (Tux in colour) in greyscale at 45% opacity, regaining most of their colour on hover and focus, not links, focusable (`tabindex="0"`, `role="img"`, `aria-disabled`, labels such as "macOS, coming soon") with a CSS-only "Coming soon" tooltip on hover and focus. Entrance: each line rises 14 px and un-blurs, staggered, once (CSS only).
2. **The live app** (`HeroApp.astro`, `lib/appdemo.ts`, `lib/appmarkup.ts`, `lib/herodemo.ts`). The desktop app's window rebuilt in HTML and CSS from its source (TitleBar, Sidebar, list Row, DetailsPane, FileMap, Sparkline and AddSheet in `apps/desktop/src`, light-theme values from `packages/ui`), overlapping the foot of the hero panel in a thin glass bezel. It is laid out at 1280 x 780 and scaled as one piece to its width (`transform: scale(tan(atan2(100cqw, 1280px)))` in a container with the same aspect ratio; a ResizeObserver sets the factor where that CSS is not supported), so on a phone it shrinks faithfully instead of reflowing. At build time it is drawn in the lively state of the screenshot it replaced: three active downloads, one paused, seven finished "Earlier today", big_buck_bunny selected with its stats, file map (an SVG still), speed graph and connections all taken from a seeded `lib/sim.ts` run, so without JavaScript it is complete (only the finished rows' times are left blank; the script fills them in as times earlier today). Above it, "Try it: pick a file" and three buttons (Linux ISO, 4K film, Game build; shown only with JavaScript). A pick plays: the Add download button presses, the add sheet slides in and the link pastes in, the probe finds the file, Download; a row appears at the top of Active and is selected, the pane follows it, the file map (a canvas ported from `FileMap.svelte`) fills square by square while the speed, the graph and the connection list move four times a second, connections join and take over parts; it ends Done with the pane's "SHA-256 verified" check, then moves to the top of Earlier today with the time it finished. Each file runs on its own seeded simulation sped up to about 8.5 s (`paceOf`), with the speeds a real line would show; the caption under the window says "A simulation of the app, sped up." The first file plays once by itself 2 s after load (after the entrance) if nothing was picked; a new pick finishes the one playing at once. The simulation advances only while the window is on screen and the tab is visible. Under reduced motion nothing plays by itself and a pick shows the finished result at once; `?at=` (review freeze) also skips the autoplay. For assistive technology the figure is labelled as a demonstration, the window is `inert` and `aria-hidden` (no live numbers are announced), and the three buttons are the only controls. The screenshot it replaced, `src/assets/app-light.webp` (captured by `apps/web/scripts/app-screenshot.mjs`), is kept as the reference for the recreation; when the app's look changes, recapture it and update the recreation to match.
3. **Speed** (pearl wash). Headline "Many connections. One file.", serif lede. The card (`SpeedCard.astro`): a white panel on a pearl wash with the file name, a status word (Reading the server, Downloading, Done) and the file map from `lib/chunkmap.ts` playing the seeded `lib/sim.ts` run at 0.7x, looping while on screen; no counters, offsets or rulers. Without JavaScript and under reduced motion the finished map is an SVG drawn at build time. Below: three points (Reads the server first, Splits the file, Finished parts take over) and a quiet "How we measure" link to `/benchmarks`.
4. **Resume** (mist wash). Headline "Pull the plug. It picks up where it stopped." The card (`ResumeCard.astro`) is three download rows in the style of the app's list; the first runs, the power is cut (the row dims, a "Power cut" chip appears), it shows as paused (the word in the paused amber), then picks up from the same point (a tick marks it) and finishes. CSS keyframes on a 12 s loop, started while on screen; the still frame (no JavaScript, reduced motion) is the moment after the resume. No numbers or hashes.
5. **Details** (paper). Headline "The details, handled.", then six points in the three-column grid: Queues, Speed limits, Link refresh, Proxy, Command line, Verified files; then one plain line "Coming next: browser extension and video downloads."
6. **Closing.** The pixel arrow (`ClosingMark.astro`, in the Onyx tile's tones and a soft ink shadow) plays its row-by-row signature once on view, then the headline "Faster from the first byte.", one serif line and the Download for Windows and Read the docs buttons.

Copy rules on Home: no dates, hardware, run counts, version numbers or competitor names; "free" and "open source" at most once on the page (the footer's "Open source on GitHub") and never in a headline.

Home's JavaScript, measured on the build: about 12 KB gzipped in all (the hero app 6.6 KB, the simulation it shares with the Speed card 1.6 KB, the Speed card 1.9 KB, link prefetch 1.1 KB, inline scripts 0.7 KB), with no Svelte runtime, inside the 15 KB target.

### Character

- **Frame.** The body is white; everything sits in one rounded panel (24 px radius, 20 on phones; fill `--graphite-50`; a 1 px hairline as `box-shadow: 0 0 0 1px rgb(10 10 11 / 0.045)`), leaving a white gutter of 16 px on phones to 24 px on desktop. The footer is the foot of the same panel: logo, one line, three short link columns and "© 2026 Boltget".
- **Header.** Like cap.so: at rest a quiet row on the white page above the frame, as wide as the frame (no fill, no shadow): the logo (the ink header lockup, `brand/svg/lockup-header-on-light.svg`, drawn for its 28 px height with an `@2x` file in `srcset`; the footer and the docs header use the same lockup), Benchmarks, Docs, Changelog (Benchmarks and Changelog hide below 736 px), GitHub with its icon, and a black Download pill. It is sticky; once the page scrolls 12 px a tiny inline script adds `.scrolled` and it narrows into the compact floating bar (60 rem wide at most, 18 px radius, white at 97% with a backdrop blur, hairline shadow), easing over 0.45 s (instant under reduced motion). It keeps its `view-transition-name`. Note for CSS: list `-webkit-backdrop-filter` before `backdrop-filter`, otherwise the minifier keeps only the prefixed one.
- **Type.** Three families: Geist 400/500/600 for headings and UI, Source Serif 4 400 and 400 italic for ledes and the italic accent line, Geist Mono 400 for code only. Headlines are Geist 400 with −0.03 em tracking (hero `clamp(2.6rem, 5.2vw, 4rem)`, sections `clamp(2.1rem, 4vw, 3.25rem)`); ledes are Source Serif 4 at about 19 px in ink at 78%; labels are small Geist 500.
- **Section rhythm.** Centred headline, a two- or three-line serif lede whose key phrases (marked `[like this]` in `en.json`, rendered by `lib/rich.ts`) take full ink against the lede's 78% ink, so they read as quiet emphasis, one large rounded card (the section's wash fading to white), then a borderless three-column grid of "**Bold lead.** plain sentence." points, each under a thin hairline. No label pills and no icon chips: the owner judged them decoration. Washes, all without hue: pearl (`.tint-pearl`, Speed), mist (`.tint-mist`, Resume), paper (`.tint-paper`, Details).
- **Icons.** `components/Icon.astro`: Lucide-style 24 px stroke icons (path data after Lucide, ISC) plus the filled GitHub and Windows marks in one colour, always `aria-hidden`. The coloured platform logos are in `components/PlatformLogo.astro` (Windows 11 four panes, the Apple logo after Simple Icons, CC0, and a Tux drawn for the site to read at 16 to 18 px). Only icons a page uses are kept.
- **Motion.** The hero's entrance, blocks fading up and un-blurring as they scroll in (`data-reveal`, only when JavaScript runs and motion is allowed), the sky's drift, and the two card loops, which run only while on screen. Cross-document view transitions (`@view-transition { navigation: auto; }`) cross-fade pages while the header and logo keep their place (`view-transition-name`); Astro prefetches every link on hover so the next page is ready. Under reduced motion nothing moves and the transitions are instant.
- **Removed.** The WebGL aurora (hero, night band, footer), the page-wide film grain, the drafting annotations and section numbers, and the footer's build facts line (`lib/facts.ts`).

### How the other pages inherit the system

Built so far (basic consistency): every marketing page uses the light theme (`data-theme="light"`, the `--type-*` scale, `.tlink`, `.statement`, pill buttons, serif ledes) inside the frame, with the header above it and the light footer, so `/download`, `/benchmarks`, `/compare/idm` (redirect), `/changelog`, `/privacy` and the 404 page match Home; their content was not redesigned yet. Starlight already uses Geist and the monochrome accent (ink on light, white on dark, graphite greys; `src/styles/starlight.css`) and is otherwise unchanged; its code blocks still use Expressive Code's default syntax colours. The plan below remains the target.

- **/benchmarks**: paper theme, a short atmospheric band at the top (the same shader, 40% height, already dissolved), then the figures in the big figure style and the result tables in Geist Mono with hairlines. Each target gets a small static file map showing its median connection count.
- **/compare/idm**: paper theme, no atmosphere (a factual page), the comparison table with the "Available now"/milestone status as pills, the measured table in mono.
- **/changelog, /privacy, 404**: paper, statement type for the page title, body at 17 px, 40 rem measure.
- **Docs (Starlight)**: keep the reader's theme; light uses paper tokens and an ink accent, dark uses ink with a white accent with the map squares as a faint header texture. Headings use the display tracking rules; code blocks use the demo's terminal palette.
- **OG images**: a still of the atmosphere with the chunk dissolve and the page title in display type.

### Tech choices

| Need | Choice | License |
|---|---|---|
| Hero sky | The pearl sky, a still from the brand's source art (`brand/source/sky-master.webp`); before it, a generated still (`scripts/sky.mjs`: seeded fBm through `sharp`), and in October a hand-written WebGL shader | project (GPL-3.0-or-later) |
| Demos | Svelte 5 islands (already in the stack); canvas 2D for maps; SVG for the graph; `svelte/transition` for list rows | MIT |
| Sitemap control | `@astrojs/sitemap` 3.7.4, registered explicitly so `/lab/` can be filtered out (Starlight would otherwise add it unfiltered) | MIT |
| Considered, not used | GSAP: its "Standard no-charge license" is not an open-source license and restricts some uses, so it is not GPL-compatible; Motion (MIT) and Lenis (MIT) are compatible but unnecessary (CSS and Svelte cover the motion; smooth-scroll hijacking hurts accessibility); OGL/three (MIT) are unnecessary for one full-screen triangle | — |

### Performance budget for the redesign

The budget below still applies. Specific targets: hero shader ≤ 4 KB, demo island ≤ 12 KB gzipped plus the Svelte runtime (≈ 12 KB), so Home stays near the 15 KB design target only if the demo is rewritten imperatively (as the current hero is) or Svelte is dropped for the demo; the decision is part of phase 2. The canvas renders at most at DPR 2, the field pauses off screen, the demo updates the DOM at ≤ 30 fps. LCP is the headline text (no image), CLS 0 (the card reserves its height). Contrast: text on the field must sit on the darker stops (white on `#046C52` is about 6.5:1); in the prototype the sentence can cross the brighter curtain, where it drops to about 4:1, so the full build clamps the ramp behind the copy. On paper, muted text is `#52525B` (7.7:1).

### Build order

1. **Decide** between A and B on the lab pages: **done, A** (October 2026).
2. **System**: paper/ink/atmosphere tokens into `brand/scripts/build.mjs` (so they are generated, not hand-written), the type scale and spacing as CSS custom properties, the shared header and footer in the new style, the `tlink` link style.
3. **Hero**: move the shader and the demo out of `src/lab/` into components, copy into `en.json`, island size work (imperative rendering or a smaller runtime), reduced-motion and no-JS states, visual tests of seeded frames (`?at=` freezes the demo).
4. **Sections**: statement with the block-map scene, proof figures from bench data, resume vignette, features, CLI, closing.
5. **Inherit**: /benchmarks, /compare/idm, changelog/privacy/404, Starlight theme, OG template.
6. **Harden**: Lighthouse CI against the budget, contrast audit of every state of the demo, phone pass at 360–430 px, remove `/lab`.

### Design lab

`/lab/a` and `/lab/b` (`apps/web/src/pages/lab/`, components in `apps/web/src/lab/`) are working prototypes of the hero and first scroll for review. They use their own layout (`src/layouts/Lab.astro`), are `noindex, nofollow`, are filtered out of the sitemap in `astro.config.mjs`, and are not linked from the site. A pill at the bottom switches between them. `?at=<seconds>` freezes the demo at that moment of its loop for screenshots and review. They are removed when the redesign ships.

### Superseded: first hero concept

Kept for the record. The first concept put the giant pixel arrow on near-black, lit it row by row, and flowed its pixels into a segmented progress bar under the headline; three proof points, a feature grid and a CLI block followed. It was replaced by direction A in October 2026; its components (`HeroStage.svelte`, `PixelGlyph.astro`) were removed and remain in the git history.

## Implementation status

Updated November 2026 for the Windows desktop preview and the framed website redesign.

**Built:** `/` (direction A, see [Home as built](#home-as-built)), `/download` (see [Download page](#download-page)), `/benchmarks`, `/compare/idm` (redirect), `/changelog`, `/privacy`, a 404 page, `/.well-known/security.txt`, `robots.txt` and a sitemap, and Starlight docs under `/docs` (overview, install, using the CLI, exit codes and JSON).

**Design lab:** removed after the direction was selected; the unused Svelte integration was removed too.

**Not built yet:** `/features`; generated per-page OG images (every page uses `brand/social/og-image.png`); Lighthouse CI; security headers (HSTS, CSP); ESLint and Prettier for `apps/web`; the CLI reference generated from clap (the CLI docs pages are written by hand and point to `--help` as the reference).

Decisions:

- **Workspace.** A pnpm workspace at the repository root (`apps/*`, `packages/*` and `brand`, one `pnpm-lock.yaml`), Node pinned by `.node-version`. `brand/` is a member so `cd brand && pnpm install` keeps working; it no longer has its own lockfile. Dependency versions are pinned exactly.
- **Tokens and assets.** No `packages/ui` yet: the site imports `brand/tokens/tokens.css` and Geist Mono from `brand/fonts` directly (Geist and Source Serif 4 are subset copies in `apps/web/src/fonts`), and `apps/web/scripts/copy-brand.mjs` copies the favicons, web manifest and OG image from `brand/` into `public/` before every build (the copies are git-ignored). `packages/ui` now exists for the desktop app (generated from the same `brand/tokens/tokens.json`); moving the site onto it is a later step.
- **Primary link before an installer exists.** "Try the CLI preview", as an underlined text link; the v0.2 app is announced by the hero card's "Preview · v0.2 app" chip and caption. Since the v0.1.0 CLI release (October 31, 2026) it links to `/download` (before that, to `/docs/getting-started/`, build from source); the closing section's link and the footer's new "Download" link (Product column) do the same, and the footer's docs link is now "Install". The label became "Download for Windows" with the app's first preview (v0.2), as black pill buttons in the hero, the header ("Download") and the closing section. `/download` is not in the header: a fourth link overflowed the bar at 390 px, and three links keep it calm.
- **Simulation.** `src/lib/sim.ts` (`DEMO_SIM`: 420 MiB, start 4, add one every 0.6 s while the last addition raised throughput by more than 6%, a wobbling 58 MiB/s line cap, steals on finish), seeded (mulberry32) with fixed 1/120 s steps. Live numbers are `aria-hidden`; each figure's accessible name says it is a simulation.
- **Pixels.** `chunkmap.ts` eases squares in on the wall clock, not on simulation time, so a scene held still still finishes its easing; a draw that is not part of a running animation (a still, `?at=`, a resize) settles at once.
- **Fonts.** Self-hosted Latin subsets (Latin-1, punctuation, figures; about 13 KB per Geist weight and 20 KB per Source Serif 4 style) in `apps/web/src/fonts/`, taken pre-subset from Fontsource (`@fontsource/geist` 5.3.0, `@fontsource/source-serif-4` 5.3.1) with their OFL licences beside them; Geist Mono still comes from `brand/fonts`. Only Geist 400 is preloaded. Characters outside the subset (an arrow such as U+2192) would fall back to a system font, so arrows on the site are icons.
- **JavaScript on Home (November 2026).** About 5.6 KB gzipped: the Speed card with `sim.ts` and `chunkmap.ts` 3.6 KB, Astro's prefetch script 1.1 KB, and about 1 KB of small inline scripts (scroll reveals, the resume card, the closing mark, the `?at=` review aid). Same method as below.
- **JavaScript on Home (October 2026, superseded).** About 13.6 KB gzipped in total, under the 15 KB design target, and no Svelte: the hero card (`AppDemo.astro`) is static markup rendered at build time from `lib/appdemo.ts` (the finished download, so the first frame is in the HTML and CLS stays 0), animated by a plain script. Breakdown: the hero script 2.4 KB (with `appdemo.ts`), `sim.ts` 1.6 KB, `chunkmap.ts` 1.3 KB, the shader 2.9 KB, the How-it-works scene 2.7 KB, and about 2.7 KB of small inline scripts (Measured, resume card, tiles, closing mark, the `?at=` review aid). Measured as the gzip (level 9) size of every script `dist/index.html` loads, followed through imports; by the same method the Svelte island version was 34.0 KB (Svelte runtime 15.5 KB, island loader and helpers about 2.7 KB). Every scene, the hero included, is plain script that pauses off screen and in hidden tabs; the Svelte integration stays for the `/lab/` pages.
- **JavaScript on the other marketing pages.** Only Astro's hover prefetch (1.1 KB gzipped) and the 0.2 KB `?at=` review aid; the footer's aurora shader is gone.
- **Marketing pages use the paper theme.** The docs follow the reader's theme (Starlight's toggle) with the brand's light tokens.
- **Tokens.** The atmosphere ramp is generated by `brand/scripts/build.mjs` as `--atmosphere-*` (and `atmosphere` in `tokens.json`); the type scale, spacing and paper hairlines are custom properties in `src/styles/global.css`.
- **Benchmark data.** `/benchmarks`, the Fast proof point read the newest `bench/results/<date>-*/` folder (`summary.csv`, `environment.json`, `targets.toml`) at build time, so a new run folder updates the site without code changes. Medians and spreads are shown with 2 decimals and MiB/s with 1, as in [BENCHMARK.md](BENCHMARK.md). A difference is stated only when the min-to-max spreads of IDM's default and Boltget's adaptive default do not overlap, computed from the data.
- **Comparison.** The separate comparison and its data were retired; existing links redirect to benchmarks.
- **Copy.** Short strings are in `apps/web/src/i18n/en.json` (keyed, whole sentences with named placeholders, plural forms for counts). Long-form copy and data are content collections (`src/content/pages`, `src/content/docs`, `src/data`). `/changelog` renders the root `CHANGELOG.md` through a content collection.
- **security.txt** expires on 2027-10-28 and must be renewed before then.

## Performance budget

| Metric | Budget |
|---|---|
| Lighthouse (mobile, simulated throttling) | 100 in Performance, Accessibility, Best Practices, SEO on every page (target; planned CI thresholds: 95 for Performance, 100 for the others) |
| JavaScript on Home | Hero island only; design target ≤ 15 KB gzipped |
| JavaScript on other marketing pages | 0 KB by default, apart from the hover prefetch (about 1 KB) |
| Docs pages | Starlight's own budget (search is loaded on demand) |
| Total transfer for first view of Home | Design target ≤ 200 KB, including fonts and hero |
| Fonts | Self-hosted WOFF2, subset to the characters needed; three families: Geist 400/500/600, Source Serif 4 400 and 400 italic, and Geist Mono 400 for code only; `font-display: swap`; the critical one preloaded |
| Images | Astro `<Image>`/`<Picture>` with AVIF/WebP, explicit dimensions, lazy below the fold; the hero sky is a generated still under 30 KB and the app screenshot loads eagerly |
| LCP | < 1.5 s on the Lighthouse mobile profile (design target) |
| CLS | 0 (layout reserved for every image and the hero) |
| Third-party requests | None. No analytics scripts, no tag managers, no embedded videos from third parties, no external fonts. |

Analytics, if ever needed, must be privacy-respecting, cookieless and server-side or a first-party script well under 2 KB, and documented on `/privacy`.

Planned: Lighthouse CI on every PR touching `apps/web` against the preview build, failing on budget regressions. Current Web CI runs Astro check and a static build.

## Download page

### As built (November 2026)

The Windows installer is the primary download. Platform availability, system requirements, an illustrated SmartScreen explanation and first-run instructions lead into the secondary CLI assets and SHA-256 verification. Release metadata is fetched at build time, with a GitHub Releases fallback when unavailable. Redeploy after publishing a release.

### Previous implementation (v0.1, superseded)

`apps/web/src/pages/download.astro`, copy under `download.*` in `en.json`, no JavaScript of its own (paper theme, the footer's aurora is the only script).

- **Release data at build time.** `src/lib/releases.ts` lists `/repos/boltget/boltget/releases` (because `/releases/latest` skips pre-releases) and takes the newest release that is not a draft, its assets and the contents of its `SHA256SUMS` (falling back to the `digest` GitHub records per asset). It sends `GITHUB_TOKEN` when set (higher rate limit) and is otherwise unauthenticated. If the API is unreachable, rate-limited or returns an error, the build logs a `[releases]` warning and still succeeds: the page then shows "Download from GitHub" (the releases page), no file list and no checksums, and the commands use `X.Y.Z` placeholders. `BOLTGET_RELEASES_OFFLINE=1` skips the network (offline builds, checking the fallback).
- **Content.** Label with version, "Preview" for a pre-release, and date; the Windows x64 zip as the one primary button with its file name and size; Linux x64 as a text link marked "preview build"; release notes. Files: platform, file name, size and SHA-256 per asset, plus `SHA256SUMS` and all releases. Verify: `Get-FileHash`, `sha256sum -c`, `gh attestation verify`, the not-signed note (SmartScreen) and a link to [BUILDING.md](BUILDING.md). Quick start in three lines (`tar -xf`, `cd`, `.oltget get <url>`) linking to `/docs/cli/`. What comes next: desktop app v0.2 (in progress), browser extension v0.3, macOS after 1.0, one line saying an installer, winget and the store listings arrive with those releases, and the roadmap link.
- **Deviations from the plan below.** No OS detection yet: Windows is the only supported platform in v0.1, so the primary button is always Windows and Linux is shown as a secondary link (detection returns with the app installers). No winget command and no store badges, because neither exists. The page reflects the release as of the last build: the site is deployed by hand (`pnpm --filter @boltget/web build`, then `pnpm --filter @boltget/web run deploy`), so **redeploy the site after publishing a release**; the deploy hook below comes with automatic deploys.

### Plan

- Detects the OS from the user agent (and Client Hints where available) and shows the right primary button; always shows all options below.
- Assets are linked from GitHub Releases (the latest stable release), resolved at build time from the GitHub API and refreshed on each release by a deploy hook, so the page never links to a stale version.
- Shows SHA-256 checksums and links to the signature/attestation (see [RELEASE.md](RELEASE.md#reproducibility-and-provenance)).
- Shows `winget install Boltget.Boltget` (package identifier to be confirmed when submitted).
- Extension store badges for Chrome, Edge and Firefox.
- Before macOS/Linux builds exist: a clear "coming after 1.0" note with a link to the roadmap.

## Compare page (retired)

The separate comparison page and its data were removed in November 2026. `/compare/idm` redirects to `/benchmarks/`. The following is the historical plan.

- A fair table derived from [FEATURES.md](FEATURES.md) (including rows where IDM is better).
- Benchmark summary with a link to the full methodology and raw data.
- "Moving from IDM" guide: concepts map (categories, queues, scheduler), and settings import if it exists by then.
- Dated ("Last reviewed: <date>, IDM version <x>"). Reviewed before each Boltget minor release.
- Tone: confident, factual, no mockery of IDM. IDM is a good product; the page shows measured results and lets them speak.

## OG images and metadata

- Every page has a title, description, canonical URL and Open Graph/Twitter card tags, generated from frontmatter.
- OG images (1200×630) are generated at build time from a template: near-black background, the pixel arrow, the page title in the brand typeface. Generated with a build-time renderer (for example Satori + resvg, or an Astro OG integration), so there is no runtime image service.
- The Home OG image is a static frame of the hero (arrow fully lit, segmented bar).
- Favicons and app icons are generated from [brand/icon.svg](../brand/icon.svg); the SVG favicon is used where supported, with PNG fallbacks.

## Docs (Starlight)

Changelog and privacy share `ReadingPage.astro`: the existing framed shell, a readable content column and a sticky section index on wide screens. On narrow screens the index follows the article. Changelog content and heading anchors come directly from the root `CHANGELOG.md`; privacy describes the current desktop preview separately from the planned extension. The 404 page uses the same shell and a direct return-home action.

Starlight keeps its native navigation, search and light/dark theme toggle, with the shared Geist fonts. The overview and install guide now start with the Windows app. The remaining documentation expansion below is planned.

- Mounted at `/docs`. Sidebar sections: Getting started, Browser extension, Downloads and queues, Video, CLI, Settings reference, Troubleshooting, FAQ.
- The CLI reference is generated from the CLI's `--help` output / clap definitions at build time so it cannot drift.
- Search via Starlight's built-in static search (Pagefind), loaded only when the user opens search.
- Themed with Boltget tokens from `packages/ui` (matching the app).

## Hosting

**Decided (2026-10-30): Cloudflare Workers with static assets** (Cloudflare's successor to Pages), free plan. The Worker `boltget-web` has no code: `apps/web/wrangler.jsonc` serves `apps/web/dist` with `404.html` for unknown paths and trailing-slash handling, on the custom domains `boltget.app` and `www.boltget.app` (the `boltget.app` zone and its DNS are on Cloudflare; the domain is registered at name.com). The site stays fully static, so the host can still change without code changes; the Astro Cloudflare adapter is deliberately not used.

Deploy by hand for now: `pnpm --filter @boltget/web build`, then `pnpm --filter @boltget/web run deploy` (Wrangler, signed in to the project's Cloudflare account). Automatic deploys from `main` and per-PR previews come later and need a `CLOUDFLARE_API_TOKEN` secret in GitHub Actions.

Requirements regardless of host:

- HTTPS only, HSTS, and a strict Content Security Policy (no inline scripts except Astro's hashed ones).
- Preview deployment for every PR touching `apps/web`, linked from the PR.
- `security.txt` at `/.well-known/security.txt` pointing to [SECURITY.md](../SECURITY.md)'s reporting channel.
