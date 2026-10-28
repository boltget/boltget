# Website

boltget.app is the product's front door: it explains BoltGet in seconds, proves the speed claim, gets the installer onto the user's machine, and hosts the user documentation.

- Code: `apps/web` (planned)
- Stack: Astro (static output) + Svelte 5 islands, user docs with Astro Starlight in the same project, shared components and tokens from `packages/ui`. Rationale: [TECH-STACK.md, ADR-010](TECH-STACK.md#adr-010-astro-and-starlight-for-the-website-and-docs).
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

### Hero concept

The hero shows what BoltGet does, not a stock illustration of speed.

1. **The giant pixel arrow.** The BoltGet mark at hero scale on near-black, built from the same 9×9 grid of square pixels with small gaps. On load, it plays the signature motion once: rows light up top to bottom in the mint→emerald gradient, like chunks arriving.
2. **The arrow becomes a download.** As the last row lights, the pixels flow into a horizontal progress bar under the headline, which then plays a **live segmented-download visualization**:
   - The bar starts split into a few segments, each filling from its left edge at slightly different rates.
   - When a segment finishes, the largest remaining segment visibly splits in two and a new "connection" starts on the second half: dynamic segmentation, explained in four seconds without a word.
   - A **speed counter** with tabular figures ticks alongside, and a connection count.
3. **Honesty rule:** this is a simulation of the algorithm, not a real download. The counter is labelled as such in small text ("Simulated. See real benchmarks"), linking to `/benchmarks`. The site never presents invented numbers as measurements.

Copy above it: one headline, one sentence, one primary button ("Download for Windows"), one secondary link ("See the benchmark"). Headline: the tagline, *"Every byte, at full speed."* Sentence, final wording TBD: *"Splits every file across many connections and keeps each one busy until the last byte."* Hero copy follows the voice rules in [brand/BRAND.md](../brand/BRAND.md#8-voice-and-tone): lead with what BoltGet does, no "free" or "open source" in the hero, no comparisons with other products.

Behaviour:

- **Reduced motion:** the arrow renders fully lit and the bar renders as a static, completed segmented bar with a label. No animation.
- **No JavaScript:** the static SVG arrow and the headline render; the visualization is progressive enhancement.
- **Implementation:** one Svelte island using a single `<canvas>` (or SVG with transforms), started with `client:visible`, pausing when off screen or when the tab is hidden. Deterministic seeded randomness so the visualization looks the same for every visitor and can be visually tested.

### Below the hero

Three proof points, each one sentence plus a small visual: **Fast** (measured throughput from the benchmark, linking to `/benchmarks`, where the comparisons live), **Never loses progress** (crash-and-resume), **Catches downloads from the browser** (the extension). Then a short feature grid, a screenshot of the app in dark theme, and the download button again. The license and GitHub link sit once, low on the page (footer). That is the whole page.

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
- Tone: confident, factual, no mockery of IDM. IDM is a good product; BoltGet is a better deal.

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
