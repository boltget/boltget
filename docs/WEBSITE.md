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

The first version of Home (a centred headline over the giant pixel arrow, a segmented bar below it, then card grids) is built and described under [Implementation status](#implementation-status). The project owner judged it correct but not captivating, so Home is being redesigned. This section is the plan; two prototypes of it live at `/lab/a` and `/lab/b` (see [Design lab](#design-lab)).

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

1. **How it works** (statement): kicker + one 3-line statement in display-weight type: *"It starts with four connections and adds more only while they make the download faster. When one finishes early, it takes over half of the largest part left, so every connection finishes together."* Full build: B's block map plays here, full bleed on ink, as the scene that *shows* the statement (scroll-triggered, one run, then rests).
2. **Proof**: three big figures in a row, read from `bench/results/` at build time: lab 1 GiB median MiB/s, public CDN median MiB/s, "1,000 restarts" (kill-and-resume test), each with one sentence and one link to the method. Never a number that is not in the data.
3. **Never loses progress**: a small live crash-and-resume vignette (the terminal: Ctrl-C, `boltget resume 7`, continues at 63%) beside one sentence.
4. **Built into the engine**: the six engine features as a calm two-column list with hairline separators, no cards, no icons.
5. **For scripts**: the CLI block (four real commands) on paper, mono, with the exit codes/JSON link.
6. **Closing**: a single large line *"Build it, run it, measure it."* with two text links, then the footer, where the license line (the one mention of free/open source) lives.

### How the other pages inherit the system

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

1. **Decide** between A and B (or the recommended combination) on the lab pages; record the decision here.
2. **System**: paper/ink/atmosphere tokens into `brand/scripts/build.mjs` (so they are generated, not hand-written), the type scale and spacing as CSS custom properties, the shared header and footer in the new style, the `tlink` link style.
3. **Hero**: move the shader and the demo out of `src/lab/` into components, copy into `en.json`, island size work (imperative rendering or a smaller runtime), reduced-motion and no-JS states, visual tests of seeded frames (`?at=` freezes the demo).
4. **Sections**: statement with the block-map scene, proof figures from bench data, resume vignette, features, CLI, closing.
5. **Inherit**: /benchmarks, /compare/idm, changelog/privacy/404, Starlight theme, OG template.
6. **Harden**: Lighthouse CI against the budget, contrast audit of every state of the demo, phone pass at 360–430 px, remove `/lab`.

### Design lab

`/lab/a` and `/lab/b` (`apps/web/src/pages/lab/`, components in `apps/web/src/lab/`) are working prototypes of the hero and first scroll for review. They use their own layout (`src/layouts/Lab.astro`), are `noindex, nofollow`, are filtered out of the sitemap in `astro.config.mjs`, and are not linked from the site. A pill at the bottom switches between them. `?at=<seconds>` freezes the demo at that moment of its loop for screenshots and review. They are removed when the redesign ships.

### Superseded: first hero concept

Kept for the record. The first concept put the giant pixel arrow on near-black, lit it row by row, and flowed its pixels into a segmented progress bar under the headline; three proof points, a feature grid and a CLI block followed. It is what `/` still shows today (details under Implementation status).

## Implementation status

First version, built during v0.1 (there is no installer yet, only the engine and CLI).

**Built:** `/` (the first hero concept, see [Superseded: first hero concept](#superseded-first-hero-concept); its original spec is in the git history of this file), `/benchmarks`, `/compare/idm`, `/changelog`, `/privacy`, a 404 page, `/.well-known/security.txt`, `robots.txt` and a sitemap, and Starlight docs under `/docs` (overview, install from source, using the CLI, exit codes and JSON).

**Design lab:** `/lab/a` and `/lab/b`, prototypes of the redesign (see [Design lab](#design-lab)); noindex and not in the sitemap.

**Not built yet:** `/download` and `/features`; generated per-page OG images (every page uses `brand/social/og-image.png`); Lighthouse CI; security headers (HSTS, CSP), which wait for the host decision; font subsetting (the full Geist WOFF2 files are served, about 120 KB for the three weights); ESLint and Prettier for `apps/web`; the app screenshot on Home (no app until v0.2); the CLI reference generated from clap (the CLI docs pages are written by hand and point to `--help` as the reference).

Decisions:

- **Workspace.** A pnpm workspace at the repository root (`apps/*`, `packages/*` and `brand`, one `pnpm-lock.yaml`), Node pinned by `.node-version`. `brand/` is a member so `cd brand && pnpm install` keeps working; it no longer has its own lockfile. Dependency versions are pinned exactly.
- **Tokens and assets.** No `packages/ui` yet: the site imports `brand/tokens/tokens.css` and the fonts in `brand/fonts` directly, and `apps/web/scripts/copy-brand.mjs` copies the favicons, web manifest and OG image from `brand/` into `public/` before every build (the copies are git-ignored). The tokens move to `packages/ui` when the desktop app needs them too.
- **Primary button before an installer exists.** It reads "Try the CLI preview" and goes to `/docs/getting-started/` (build from source), with a status line under the buttons: "v0.1 is the engine and the command-line tool. The Windows app arrives in v0.2." It becomes "Download for Windows", linking to `/download`, when the first preview builds are published (v0.2).
- **Hero layout.** Copy on top, the arrow below it, the bar directly under the arrow, so the pixels fall straight down into the bar. The row-by-row lighting is CSS, so it also plays without JavaScript. When the last row is lit, copies of the 37 pixels fly into 37 slots across the bar and merge into it; the arrow itself stays lit. This movement is the one exception to the brand rule that only opacity animates, because this spec asks for the flow.
- **Simulation.** 320 MiB, 4 connections at the start and one more every 0.9 s up to 6 (the adaptive start of 4 is real; the rest is illustrative), a split only when at least 16 MiB remain, 10.5 to 15.5 MiB/s per connection with slow variation and a short ramp-up. Seeded (mulberry32) with fixed 1/120 s steps. A finished download holds "Done" for 2.4 s, fades, and the next one starts; the arrow does not replay. The live numbers are `aria-hidden`; the SVG's text alternative says it is a simulation.
- **JavaScript on Home.** Measured at about 17.8 KB gzipped: the island 3.5 KB, the Svelte client runtime 11.6 KB plus its 0.5 KB Astro renderer, and Astro's inline island loader 2.2 KB. That is over the 15 KB design target. The island already draws imperatively (pooled SVG rects) to keep its own share small; the rest is the Svelte runtime. Astro's link prefetching is off, so the other marketing pages ship no JavaScript.
- **Marketing pages are dark only.** The docs follow the reader's theme (Starlight's toggle) with the brand's light tokens.
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

Cloudflare Pages or Vercel; decide before v1.0. Both serve a static Astro build well, offer preview deployments per PR and custom domains. Current leaning: **Cloudflare Pages**, for generous static bandwidth on the free tier and no need for server functions. The site must remain fully static so the host can be changed without code changes.

Requirements regardless of host:

- HTTPS only, HSTS, and a strict Content Security Policy (no inline scripts except Astro's hashed ones).
- Preview deployment for every PR touching `apps/web`, linked from the PR.
- `security.txt` at `/.well-known/security.txt` pointing to [SECURITY.md](../SECURITY.md)'s reporting channel.
