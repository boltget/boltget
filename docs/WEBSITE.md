# Website

boltget.app is the product's front door: it explains BoltGet in seconds, proves the speed claim, gets the installer onto the user's machine, and hosts the user documentation.

- Code: `apps/web` (package `@boltget/web`): `pnpm install`, then `pnpm --filter @boltget/web dev` (or `build`, `preview`, `check`). CI: `.github/workflows/web.yml`. What is built so far, and the decisions made while building it, are under [Implementation status](#implementation-status).
- Stack: Astro (static output) + Svelte 5 islands, user docs with Astro Starlight in the same project, shared components and tokens from `packages/ui` (until `packages/ui` exists, the site imports `brand/tokens/tokens.css` and `brand/fonts` directly). Rationale: [TECH-STACK.md, ADR-010](TECH-STACK.md#adr-010-astro-and-starlight-for-the-website-and-docs).
- Language: English only, with all copy in the site's string catalog or content collections (no copy scattered through components), so a community translation stays possible later. See [DESIGN-PRINCIPLES.md, Copy and strings](DESIGN-PRINCIPLES.md#copy-and-strings).
- Live by: v1.0 (a minimal landing page can go up earlier; see [ROADMAP.md](ROADMAP.md)).

## Site map

| Path | Page | Purpose |
|---|---|---|
| `/` | Home | The pitch in one screen; the hero; three proof points; download button |
| `/download` | Download | OS-detected primary button, all assets, checksums, winget command, system requirements, extension store links |
| `/features` | Features | The capabilities, each with a small explanatory visual, linking into docs |
| `/compare/idm` | BoltGet vs IDM | Honest comparison, the benchmark summary, migration guide link |
| `/benchmarks` | Benchmarks | Methodology, dated results per version, raw data links ([ENGINE.md](ENGINE.md#benchmarks-plan-vs-idm)) |
| `/docs/...` | Documentation (Starlight) | Getting started, browser extension setup, video downloads, CLI reference, troubleshooting, FAQ |
| `/changelog` | Changelog | Rendered from `CHANGELOG.md` at build time |
| `/privacy` | Privacy | What the app, extension and site collect (nothing, by default), in plain words |
| `/blog` | Blog | Later (post-1.0): engineering deep dives, release notes |
| `/update/{channel}/latest.json` | Update manifests | Static files consumed by the app's updater ([RELEASE.md](RELEASE.md#auto-update)); not linked from pages |

The repository's design documents (this `docs/` folder) are for contributors. The website's docs are for users. They link to each other but are not the same content.

## Home page

Home was redesigned in October 2026. The owner chose **direction A ("Daylight")** from the two prototypes at `/lab/a` and `/lab/b` (see [Design lab](#design-lab)); it is now built as `/`. This section keeps the research and the plan; what was actually built, and the decisions taken while building it, are in [Home as built](#home-as-built) and [Character](#character). The first Home (a centred headline over the giant pixel arrow) is described under [Superseded: first hero concept](#superseded-first-hero-concept).

### Research notes

What makes the reference sites feel premium, in one line each (studied October 2026):

- **contravise.co** (the owner's reference): a looping atmospheric video field (deep blue to cyan) under a dark shade that fades to white; a 60 px Manrope headline at −0.05 em tracking, two lines, left; one sentence and two underlined text links on the right, no buttons; a large white card (24 px radius, soft shadow) overlaps the bottom of the field and plays the product live when it scrolls into view: a question types itself in large type, the answer appears, the cited clause highlights in the document beside it. Below: white sections, 40–56 px headings, prices written as a sentence with underlined amounts, a quiet FAQ.
- **linear.app**: near-black, headline and product UI fade and un-blur in sequence; the product screenshot is the hero, not an illustration; tight, small UI type around one big statement.
- **raycast.com**: one centred line of huge type, almost nothing else above the fold; the product window appears as the scroll reward.
- **vercel.com**: a single geometric object lit in an otherwise black field, hairline grid lines, a calm two-column hero.
- **family.co**: light, soft, playful motion with small floating objects; proves a light page can feel as premium as a dark one.

Shared traits: very few words; one centerpiece that is the real product (or a faithful simulation of it); type that is large, tight and calm rather than loud; motion that explains rather than decorates; long quiet whitespace; text links over button stacks.

### Concept: the page is a download

A download manager's craft is invisible: bytes arrive, a file appears. BoltGet's mark already draws the idea (each pixel is a chunk). The redesign makes the whole page behave like a download: **the atmosphere is made of chunks, the centerpiece is a file arriving piece by piece, and every number is either measured or labelled as simulated.** No other download manager site looks like this, because none of them shows the thing actually happening.

Two directions were prototyped:

- **A, Daylight.** Light and atmospheric. A slow emerald-to-teal aurora fills the top of the page and, towards the bottom, breaks into square chunks with even gaps that thin out into white. A large white app card overlaps it and plays a simulated preview of the v0.2 desktop app (link typed, file probed, connections coming online, file map filling, checksum verified).
- **B, Block map.** Dark and bold. The whole hero is the map of a file: thousands of faint squares, each a slice of the file, light up as connections write them, in bright moving edges that settle to a faint emerald. In front, a terminal runs `boltget get` in the CLI's real output format, synchronised with the map behind it.

**Recommendation: A as the site's system, with B's block map as the second scene.** A is closest to what the owner asked for (light, airy, a product card as the centerpiece) and scales to every other page; B's map is the most distinctive single idea and is honest today (it shows the CLI that exists). The full build uses A's hero; the block map returns below the fold as the "how it works" scene, and becomes the dark docs/benchmarks hero accent. The owner decides; the plan below assumes this recommendation.

### Art direction

**Color.** Two surfaces: *paper* (white `#FFFFFF`, text ink `#0B0B0F`, muted `#52525B`) for the body of the page, and the *atmosphere* (deep emerald `#031E18` → `#023428` → `#046C52` → teal `#0D9488` → `#34D3AF` → mint `#A7F3D0` → `#ECFDF5` → white) only in the hero field. The accent on paper is emerald-700 `#047857` (links, kicker labels, success). The mark's gradient stays on the mark. Teal is new and allowed only inside the atmosphere and in data series (file map, connection colours); see [brand/BRAND.md, Atmosphere](../brand/BRAND.md#atmosphere-website). Dark pages (B's map, docs dark theme) use ink `#08080B` with the map squares at most 14% opacity when settled.

**Type.** Geist 400/600 and Geist Mono 400 only (three weights, the budget). Scale: display `clamp(3.4rem, 6.6vw, 6.25rem)` / 0.95 / −0.055 em; statement `clamp(1.75rem, 3.6vw, 3.25rem)` / 1.12 / −0.035 em, weight 600; figure (measured numbers) `clamp(2rem, 3.4vw, 3rem)` / 1 / −0.045 em, or Geist Mono at up to 9.5 rem for B; body 17–20 px / 1.5; UI inside the demo 12–16 px; kicker label mono uppercase 13 px, +0.12 em, emerald-700 (one per section). Headlines never exceed two lines on desktop; `text-wrap: balance` for headings, `pretty` for paragraphs.

**Grid and spacing.** 84 rem max width, gutter `clamp(1rem, 4.5vw, 3rem)`. Hero: headline left (≈ 55%), sentence and links right, bottom-aligned. Sections are separated by large vertical rhythm (`clamp(6rem, 14vw, 11rem)`) rather than rules or cards. Card radius 20 px for the demo window only; UI chips are pills; everything pixel-related (map squares, field chunks) is square, never rounded.

**Motion principles.** Motion explains a mechanism or it does not ship. (1) Entrance: headline, aside and card rise 14 px and un-blur over 1.1 s with a 0.12 s stagger (`cubic-bezier(0.22, 1, 0.36, 1)`), once. (2) The demo is the only looping motion above the fold; it pauses off screen and in hidden tabs. (3) The atmosphere moves slowly enough to read as weather, not animation (noise drift of ≈ 0.03 units/s). (4) Chunks arrive linearly, like the brand motion; no bounce or overshoot. (5) `prefers-reduced-motion`: no loop anywhere; the demo shows its finished state, the field one still frame. (6) No scroll-jacking or smooth-scroll libraries.

### Hero and the live demo (direction A)

1. **Field.** A WebGL fragment shader (≈ 3 KB, no library) draws the aurora: domain-warped fbm noise plus one slow luminous "curtain" band, mapped through the atmosphere ramp. Below 50% of its height, the field is quantised into 13 px cells (10 px on phones) with 2 px gaps; cells drop out by a per-cell hash as the field approaches white, with a slow per-cell wobble so a few chunks come and go. Behind the card, this is the visible "chunks arriving" edge. Fallback without WebGL or JS: a CSS gradient with the same stops.
2. **Copy.** Header with the on-dark lockup and four quiet links. Headline *"Every byte, at full speed."*; sentence *"Splits every file across many connections and keeps each one busy until the last byte."*; two underlined links, *Try the CLI preview* and *See the benchmark*. No buttons in the hero.
3. **The card.** A white window (title bar with the 16 px icon, "BoltGet", a **"Preview · v0.2 app"** chip, Windows caption buttons), sidebar (All downloads / Active / Completed / Scheduled with counts) and main pane. Timeline of one loop (≈ 16 s): empty state *"Paste a link or drop one here."* with a blinking caret → the link types itself (30 ms/char) → the Download button presses → "Connecting to cdn.example.com" → the job appears: file name, `420 MiB · cdn.example.com · resumable`, big tabular stats (speed, received, time left, connections), the **file map** (6 × 56 squares; each square a slice of the file, coloured by the connection that wrote it, the square each connection is writing in ink), a throughput graph drawing itself, and the connection list where rows appear as the controller adds them and show "took half of #3" when a connection steals work → **Done. 420 MiB in 7.9 s.** with "SHA-256 verified", the Completed count ticks from 11 to 12 → hold 4 s, fade, loop. "Earlier today" rows under the job keep the window from ever looking empty.
4. **Simulation.** `apps/web/src/lab/sim.ts`: seeded (mulberry32), fixed 1/120 s steps, start 4 connections, add one every 0.6 s while the last addition raised throughput by more than 6%, a wobbling 58 MiB/s link cap that makes more connections stop paying off (the demo settles at 7), work stealing on finish. It mirrors [ENGINE.md](ENGINE.md)'s behaviour, not its numbers.
5. **Honesty.** The chip says it is a preview of v0.2; the caption under the card says *"Simulated preview of the desktop app planned for v0.2. The numbers are illustrative."* with a link to the measured results. The figure's accessible name describes the simulation; the live numbers are `aria-hidden`. Until the app exists, the CLI is the primary call to action. When v0.2 ships, the card is replaced by a recording-faithful version driven by the real UI components from `packages/ui`.

### Home sections after the hero

Superseded by [Home as built](#home-as-built). The plan was a statement paragraph, three proof figures, a terminal vignette, a feature list, a CLI block and a closing line; the owner judged a first build of it "just text". The rule that replaced it: **every section shows, not tells**: a short headline, at most one line of copy, and a visual that does the explaining.

### Home as built

Components are in `apps/web/src/components/home/`; the simulation is `src/lib/sim.ts`, the shared file-map renderer `src/lib/chunkmap.ts`, the atmosphere shader `src/components/Aurora.astro`. Copy is in `src/i18n/en.json` under `home.*`.

1. **Hero.** As in the plan (field, copy, the simulated v0.2 app card `AppDemo.svelte`, hydrated `client:load`), with the file map drawn by `chunkmap.ts` in the pixel style of [brand/BRAND.md, Pixels on the web](../brand/BRAND.md#pixels-on-the-web). The connection list uses one uniform swatch, because the map is no longer coloured per connection. Contrast fix: the shader holds the ramp near 0.31 (`#046C52` to `#05725A`) behind the copy (the element marked `data-aurora-clamp`) with a soft knee and a feathered vertical transition, so the bright curtain can no longer drift behind the sentence; worst case about 5.3:1 for white text (about 4:1 in the prototype).
2. **01 / How it works** (`HowItWorks.astro`). One file map (48 x 4 squares, 24 x 4 on phones; file order top to bottom, then right, so every part is a horizontal span with a labelled bracket above it) plays the seeded run through five steps when it is on screen: **Probe, Split, Add, Steal, Verify**. A compact step indicator (five buttons, each with a progress hairline) follows along; click, tap or the arrow keys jump to a step; hovering or focusing the scene pauses it. It never reacts to scrolling: a first build was a sticky scroll-driven scene, and the owner found having to keep scrolling tiring. Under the map: a byte-offset ruler (0 B to the file size, hex offsets at both ends) and a readout (connections, received, simulated time). Under reduced motion nothing plays; each step shows its still frame when chosen. Without JavaScript the steps are a plain list.
3. **Playground** (in the same scene). A file-size slider (24 MiB to 2 GiB) and a switch "Server limits each connection to 2 MiB/s" re-run the simulation: unlimited, the shared line cap makes the controller stop at about 7 connections; limited, it keeps adding up to 16 and the run takes much longer. Each run plays in about eight seconds whatever its size; the summary states peak connections, steals and simulated time. Labelled "Simulation".
4. **02 / Measured** (`SpeedRuns.astro`), the night band. One row per target of the newest run (BoltGet adaptive, the median from `summary.csv`): a bar of squares that fills in exactly the measured median time (a real-time replay, at most 6.8 s), the seconds counting up in large tabular figures, the MiB/s appearing when the row finishes. A drafting dimension line runs over the bars ("0 s ... whole file"); the source folder is named in the section rule. Dated, links to `/benchmarks`, no comparison with other products on Home. Without JavaScript and under reduced motion the bars are full and the figures final. A Replay link appears afterwards.
5. **03 / Never loses progress** (`ResumeDemo.astro`). A small download card runs to 64%; the "power cut" flickers and dims it ("Process killed"); the bytes after the last checkpoint (56%) are shown as lost; `boltget resume 7` types itself; it continues from the checkpoint and finishes "Byte-identical" with the digest. Beside it the one real figure: **1,000** kills over 349 downloads in testing, every file byte-identical ([ENGINE.md, Testing strategy](ENGINE.md); [ROADMAP.md](ROADMAP.md)). The card is labelled "Simulated".
6. **04 / Built in** (`Features.astro`). Six tiles, each with a small live picture instead of a paragraph: speed limit (a slider throttling a stream of squares, `--limit 4M`), checksum (a SHA-256 digest resolving, then the tick), link refresh (a 403 link replaced at 97%), proxies and cookies (squares travelling through a SOCKS5 node), scripting (rolling JSON lines, `exit 0`), browser extension (a click handed to BoltGet) marked **Planned · v0.3**. The others exist in v0.1. The scheduler was left out: six tiles were enough, and a second planned item would dilute the grid. The animations are CSS plus two tiny scripted ones and run only while the grid is visible.
7. **Closing.** The pixel arrow at 216 px plays its row-by-row signature once on view, then one light sweep passes across it; headline "Build it, run it, measure it." and two underlined links (Try the CLI preview, Read the docs).

Header and footer: the nav is Benchmarks, Docs, Changelog. Compare was left out of the nav (it stays in the footer as "BoltGet and IDM"): three links keep the bar calm, and Home avoids pointing at another product. GitHub, license and star badges are not in the header; the footer's "Open source on GitHub" is the site's only mention of the project being open source. "No ads, no account, no telemetry" and similar lines were removed from the marketing pages (the privacy page still says it).

### Character

Five layers that make the site distinctive without more imagery:

- **Drafting layer.** Each Home section opens with a hairline rule carrying its index and kicker ("01 / HOW IT WORKS") on the left and a small monospace annotation on the right ("fig. 1 · file map, one square per slice", "source: bench/results/(run)"), via `SectionHead.astro`. The file map has a byte-offset ruler with hex offsets; the benchmark bars have a dimension line. Low contrast, at most one annotation per element.
- **Colour rhythm.** Paper, night, paper, aurora footer. The night band (Measured) and the footer carry the hero's smoke (the `Aurora` shader in its `night` and `calm` modes: slower, no chunks, held on the dark stops), always inside their own section. The night band fades in from paper and out again with a long smootherstep gradient (`--feather`, 160 to 240 px), full width; the footer starts under a gentle wave (two slow sines) whose edge is feathered over about 44 px. Alpha is exactly zero at every canvas edge, so no rectangle shows. Two earlier builds were rejected by the owner: crisp SVG wave edges (too hard), then smoke billowing across the boundary into the paper (read as heavy dark clouds, and showed the canvas's straight top edge at the corners). Without WebGL, a CSS gradient with an eased multi-stop mask stands in.
- **Material.** A faint film grain over the whole page (an inline SVG `feTurbulence` tile at 3% opacity, fixed, `pointer-events: none`, hidden in print), so flat white reads as paper.
- **Playground.** See Home as built, item 3.
- **Facts.** The footer's last line, in mono, is built from the repository at build time (`src/lib/facts.ts`): the workspace version from the root `Cargo.toml`, the date of the newest benchmark run, and the short commit of the build (`GITHUB_SHA` or the host's variable when set, otherwise `git rev-parse`).

### How the other pages inherit the system

Built so far (basic consistency): every marketing page uses the paper theme (`data-theme="light"`, the `--type-*` scale, `.tlink`, `.statement`), the sticky translucent paper header and the aurora footer, so `/benchmarks`, `/compare/idm`, `/changelog`, `/privacy` and the 404 page match Home; their content was not redesigned. Starlight already uses Geist and the emerald accents and is otherwise unchanged. The plan below remains the target.

- **/benchmarks**: paper theme, a short atmospheric band at the top (the same shader, 40% height, already dissolved), then the figures in the big figure style and the result tables in Geist Mono with hairlines. Each target gets a small static file map showing its median connection count.
- **/compare/idm**: paper theme, no atmosphere (a factual page), the comparison table with the "Available now"/milestone status as pills, the measured table in mono.
- **/changelog, /privacy, 404**: paper, statement type for the page title, body at 17 px, 40 rem measure.
- **Docs (Starlight)**: keep the reader's theme; light uses paper tokens and emerald-700 accents, dark uses ink with the map squares as a faint header texture. Headings use the display tracking rules; code blocks use the demo's terminal palette.
- **OG images**: a still of the atmosphere with the chunk dissolve and the page title in display type.

### Tech choices

| Need | Choice | License |
|---|---|---|
| Hero field | Hand-written WebGL shader in an Astro `<script>` (no library) | project (GPL-3.0-or-later) |
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

First version, built during v0.1 (there is no installer yet, only the engine and CLI).

**Built:** `/` (direction A, see [Home as built](#home-as-built)), `/benchmarks`, `/compare/idm`, `/changelog`, `/privacy`, a 404 page, `/.well-known/security.txt`, `robots.txt` and a sitemap, and Starlight docs under `/docs` (overview, install from source, using the CLI, exit codes and JSON).

**Design lab:** `/lab/a` and `/lab/b`, prototypes of the redesign (see [Design lab](#design-lab)); noindex and not in the sitemap.

**Not built yet:** `/download` and `/features`; generated per-page OG images (every page uses `brand/social/og-image.png`); Lighthouse CI; security headers (HSTS, CSP), which wait for the host decision; font subsetting (the full Geist WOFF2 files are served, about 120 KB for the three weights); ESLint and Prettier for `apps/web`; the app screenshot on Home (no app until v0.2); the CLI reference generated from clap (the CLI docs pages are written by hand and point to `--help` as the reference).

Decisions:

- **Workspace.** A pnpm workspace at the repository root (`apps/*`, `packages/*` and `brand`, one `pnpm-lock.yaml`), Node pinned by `.node-version`. `brand/` is a member so `cd brand && pnpm install` keeps working; it no longer has its own lockfile. Dependency versions are pinned exactly.
- **Tokens and assets.** No `packages/ui` yet: the site imports `brand/tokens/tokens.css` and the fonts in `brand/fonts` directly, and `apps/web/scripts/copy-brand.mjs` copies the favicons, web manifest and OG image from `brand/` into `public/` before every build (the copies are git-ignored). The tokens move to `packages/ui` when the desktop app needs them too.
- **Primary link before an installer exists.** "Try the CLI preview", to `/docs/getting-started/` (build from source), as an underlined text link; the v0.2 app is announced by the hero card's "Preview · v0.2 app" chip and caption. It becomes "Download for Windows", linking to `/download`, when the first preview builds are published (v0.2).
- **Simulation.** `src/lib/sim.ts` (`DEMO_SIM`: 420 MiB, start 4, add one every 0.6 s while the last addition raised throughput by more than 6%, a wobbling 58 MiB/s line cap, steals on finish), seeded (mulberry32) with fixed 1/120 s steps. The lab's `src/lab/sim.ts` re-exports it. Live numbers are `aria-hidden`; each figure's accessible name says it is a simulation.
- **Pixels.** `chunkmap.ts` eases squares in on the wall clock, not on simulation time, so a scene held still still finishes its easing; a draw that is not part of a running animation (a still, `?at=`, a resize) settles at once.
- **JavaScript on Home.** About 33 KB gzipped in total: the Svelte runtime 15.9 KB, the hero island 4.3 KB, `sim.ts` 1.6 KB, `chunkmap.ts` 1.3 KB, the shader 2.5 KB, the How-it-works scene 2.8 KB, and about 3.8 KB of small inline scripts (Measured, resume card, tiles, closing mark, Astro's island loader). That is over the 15 KB design target; the next step is rendering the hero card without Svelte. Only the hero hydrates on load; every other scene is plain script that starts when visible and pauses off screen and in hidden tabs.
- **JavaScript on the other marketing pages.** The footer's aurora is the shader script (2.5 KB gzipped), so those pages are no longer at 0 KB; it runs only while the footer is on screen. One WebGL context per field (at most three on Home), each paused off screen; under reduced motion each draws one still frame.
- **Marketing pages use the paper theme.** The docs follow the reader's theme (Starlight's toggle) with the brand's light tokens.
- **Tokens.** The atmosphere ramp is generated by `brand/scripts/build.mjs` as `--atmosphere-*` (and `atmosphere` in `tokens.json`); the type scale, spacing and paper hairlines are custom properties in `src/styles/global.css`.
- **Benchmark data.** `/benchmarks`, the Fast proof point and the comparison read the newest `bench/results/<date>-*/` folder (`summary.csv`, `environment.json`, `targets.toml`) at build time, so a new run folder updates the site without code changes. Medians and spreads are shown with 2 decimals and MiB/s with 1, as in [BENCHMARK.md](BENCHMARK.md). A difference is stated only when the min-to-max spreads of IDM's default and BoltGet's adaptive default do not overlap, computed from the data.
- **Comparison.** The rows are in `apps/web/src/data/compare-idm.json`, derived from [FEATURES.md](FEATURES.md), each marked "Available now" or with its milestone; "Unknown" stands for FEATURES.md's `?`. "Last reviewed" shows the date and IDM version of the benchmark run, so both are reviewed together.
- **Copy.** Short strings are in `apps/web/src/i18n/en.json` (keyed, whole sentences with named placeholders, plural forms for counts). Long-form copy and data are content collections (`src/content/pages`, `src/content/docs`, `src/data`). `/changelog` renders the root `CHANGELOG.md` through a content collection.
- **security.txt** expires on 2027-10-28 and must be renewed before then.

## Performance budget

| Metric | Budget |
|---|---|
| Lighthouse (mobile, simulated throttling) | 100 in Performance, Accessibility, Best Practices, SEO on every page (target; CI fails below 95 for Performance, below 100 for the others) |
| JavaScript on Home | Hero island only; design target ≤ 15 KB gzipped |
| JavaScript on other marketing pages | 0 KB by default |
| Docs pages | Starlight's own budget (search is loaded on demand) |
| Total transfer for first view of Home | Design target ≤ 200 KB, including fonts and hero |
| Fonts | Self-hosted WOFF2, subset to the characters needed, at most two families and three weights in total; `font-display: swap`; the critical one preloaded |
| Images | Astro `<Image>` with AVIF/WebP, explicit dimensions, lazy below the fold; the hero is SVG/canvas, not a bitmap |
| LCP | < 1.5 s on the Lighthouse mobile profile (design target) |
| CLS | 0 (layout reserved for every image and the hero) |
| Third-party requests | None. No analytics scripts, no tag managers, no embedded videos from third parties, no external fonts. |

Analytics, if ever needed, must be privacy-respecting, cookieless and server-side or a first-party script well under 2 KB, and documented on `/privacy`.

CI runs Lighthouse CI on every PR touching `apps/web` against the preview build, and fails on budget regressions.

## Download page

- Detects the OS from the user agent (and Client Hints where available) and shows the right primary button; always shows all options below.
- Assets are linked from GitHub Releases (the latest stable release), resolved at build time from the GitHub API and refreshed on each release by a deploy hook, so the page never links to a stale version.
- Shows SHA-256 checksums and links to the signature/attestation (see [RELEASE.md](RELEASE.md#reproducibility-and-provenance)).
- Shows `winget install BoltGet.BoltGet` (package identifier to be confirmed when submitted).
- Extension store badges for Chrome, Edge and Firefox.
- Before macOS/Linux builds exist: a clear "coming after 1.0" note with a link to the roadmap.

## Compare page

- A fair table derived from [FEATURES.md](FEATURES.md) (including rows where IDM is better).
- Benchmark summary with a link to the full methodology and raw data.
- "Moving from IDM" guide: concepts map (categories, queues, scheduler), and settings import if it exists by then.
- Dated ("Last reviewed: <date>, IDM version <x>"). Reviewed before each BoltGet minor release.
- Tone: confident, factual, no mockery of IDM. IDM is a good product; the page shows measured results and lets them speak.

## OG images and metadata

- Every page has a title, description, canonical URL and Open Graph/Twitter card tags, generated from frontmatter.
- OG images (1200×630) are generated at build time from a template: near-black background, the pixel arrow, the page title in the brand typeface. Generated with a build-time renderer (for example Satori + resvg, or an Astro OG integration), so there is no runtime image service.
- The Home OG image is a static frame of the hero (arrow fully lit, segmented bar).
- Favicons and app icons are generated from [brand/icon.svg](../brand/icon.svg); the SVG favicon is used where supported, with PNG fallbacks.

## Docs (Starlight)

- Mounted at `/docs`. Sidebar sections: Getting started, Browser extension, Downloads and queues, Video, CLI, Settings reference, Troubleshooting, FAQ.
- The CLI reference is generated from the CLI's `--help` output / clap definitions at build time so it cannot drift.
- Search via Starlight's built-in static search (Pagefind), loaded only when the user opens search.
- Themed with BoltGet tokens from `packages/ui` (dark-first, matching the app).

## Hosting

**Decided (2026-10-30): Cloudflare Workers with static assets** (Cloudflare's successor to Pages), free plan. The Worker `boltget-web` has no code: `apps/web/wrangler.jsonc` serves `apps/web/dist` with `404.html` for unknown paths and trailing-slash handling, on the custom domains `boltget.app` and `www.boltget.app` (the `boltget.app` zone and its DNS are on Cloudflare; the domain is registered at name.com). The site stays fully static, so the host can still change without code changes; the Astro Cloudflare adapter is deliberately not used.

Deploy by hand for now: `pnpm --filter @boltget/web build`, then `pnpm --filter @boltget/web run deploy` (Wrangler, signed in to the project's Cloudflare account). Automatic deploys from `main` and per-PR previews come later and need a `CLOUDFLARE_API_TOKEN` secret in GitHub Actions.

Requirements regardless of host:

- HTTPS only, HSTS, and a strict Content Security Policy (no inline scripts except Astro's hashed ones).
- Preview deployment for every PR touching `apps/web`, linked from the PR.
- `security.txt` at `/.well-known/security.txt` pointing to [SECURITY.md](../SECURITY.md)'s reporting channel.
