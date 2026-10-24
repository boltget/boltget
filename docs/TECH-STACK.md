# Tech stack

Every major technology choice in BoltGet, recorded as a short Architecture Decision Record (ADR): the context, the decision, why, what we rejected and the consequences. These decisions are settled; changing one requires an RFC (see [CONTRIBUTING.md](../CONTRIBUTING.md#rfcs)) that addresses the reasons below.

| # | Area | Decision |
|---|---|---|
| [001](#adr-001-rust-for-the-download-engine) | Engine language | Rust |
| [002](#adr-002-tokio-hyper-and-rustls-for-networking) | Networking | Tokio, hyper (workers), reqwest (auxiliary), rustls |
| [003](#adr-003-tauri-2-for-the-desktop-app) | Desktop shell | Tauri 2 |
| [004](#adr-004-svelte-5-typescript-and-vite-for-the-ui) | UI framework | Svelte 5 + TypeScript + Vite |
| [005](#adr-005-sqlite-via-rusqlite-on-a-dedicated-thread) | Persistence | SQLite via rusqlite |
| [006](#adr-006-wxt-for-the-browser-extension) | Extension framework | WXT, Manifest V3 |
| [007](#adr-007-native-messaging-for-browser-to-app-communication) | Extension transport | Native messaging |
| [008](#adr-008-yt-dlp-and-ffmpeg-as-on-demand-sidecars) | Video | yt-dlp + ffmpeg sidecars, on demand |
| [009](#adr-009-one-engine-crate-shared-by-app-and-cli) | CLI | Same engine crate |
| [010](#adr-010-astro-and-starlight-for-the-website-and-docs) | Website and docs | Astro + Svelte islands, Starlight |
| [011](#adr-011-pnpm-workspaces-and-a-cargo-workspace-in-one-monorepo) | Repository | Monorepo, pnpm + Cargo workspaces |
| [012](#adr-012-tauri-updater-with-github-releases) | Updates | Tauri updater + GitHub Releases |
| [013](#adr-013-azure-trusted-signing-for-windows-code-signing) | Code signing | Azure Trusted Signing |
| [014](#adr-014-gpl-30-or-later) | License | GPL-3.0-or-later |
| [015](#adr-015-english-only-product-with-a-central-string-catalog) | Language | English-only product, central string catalog |
| [016](#adr-016-windows-first) | Platforms | Windows first |

---

## ADR-001: Rust for the download engine

**Context.** The engine does high-throughput network I/O on dozens of concurrent connections, parses untrusted input, writes to disk at random offsets, and must never corrupt data. It must run inside a desktop app and a CLI, and later on macOS and Linux.

**Decision.** Write the engine in Rust.

**Why.**
- Memory safety without a garbage collector: no data races in the segment table, no use-after-free in the parsers, predictable latency.
- Excellent async networking ecosystem (Tokio, hyper, rustls) that powers production proxies and CDNs.
- Tauri's backend is Rust, so the engine embeds directly with no FFI boundary.
- Small static binaries, good Windows support, easy cross-compilation.

**Rejected.**
- **Go:** great networking and simple concurrency, but a GC runtime inside the app process, and embedding Go in a Tauri/Rust app means cgo/FFI and two runtimes. Good choice for a standalone CLI; worse for our shape.
- **C++:** IDM-class performance is achievable, but memory safety, dependency management and contributor onboarding are much harder. Network parsers in C++ are a classic source of CVEs.
- **C# / .NET:** productive and good on Windows; heavier runtime, weaker story for a tiny cross-platform CLI and for embedding into a non-.NET shell.
- **Node.js (as in many Electron download managers):** not suited for high-throughput positional disk I/O with tight memory bounds.

**Consequences.** Contributors need Rust. Compile times are longer than Go's. The engine API is Rust-native; TypeScript bindings are generated, not hand-written.

## ADR-002: Tokio, hyper and rustls for networking

**Decision.**
- **Tokio** (multi-threaded runtime) for async.
- **hyper 1.x low-level client connections** (`hyper::client::conn::http1` / `http2`) for download workers, so the engine controls exactly one TCP/TLS connection per worker.
- **reqwest** (on top of hyper) for auxiliary requests where pooling is fine: update checks, sidecar manifests, simple probes in tooling.
- **rustls** for TLS, with **`rustls-platform-verifier`** so certificate validation uses the OS trust store (Windows certificate store, including enterprise root CAs).
- HTTP/1.1 and HTTP/2 from v0.1; FTP later; HTTP/3 (quinn) post-1.0.

**Why.**
- A pooled client multiplexes HTTP/2 requests to the same origin onto one connection, which defeats segmentation (see [ENGINE.md, HTTP/2 considerations](ENGINE.md#http2-considerations)). Workers need explicit connections.
- rustls is memory-safe and fast; using the platform verifier avoids the common "works at home, fails behind the corporate proxy" problem that bundled root stores cause.

**Rejected.**
- **native-tls (SChannel on Windows):** uses the OS store natively, but behaviour differs per platform and TLS features (ALPN, session resumption tuning) are harder to control uniformly.
- **curl via bindings:** extremely capable, but a C dependency with its own connection management that would fight our scheduler.
- **reqwest for workers:** pooling and HTTP/2 multiplexing are the wrong defaults for this job.

**Open.** rustls crypto provider (`aws-lc-rs` vs `ring`) is decided in v0.1 from throughput measurements and Windows build friction.

## ADR-003: Tauri 2 for the desktop app

**Context.** We want a modern, beautiful UI built with web technology (fast iteration, shared components with the website) without the size and memory cost of shipping a browser.

**Decision.** Tauri 2, using the system WebView (WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux), with the Rust backend hosting the engine.

**Why.**
- Small installers and low idle memory compared with bundling Chromium. (Exact sizes are measured in v0.2; the target in [VISION.md](VISION.md) is under 15 MB.)
- Rust backend: the engine is a normal crate dependency.
- Mature pieces we need: tray, single instance, updater with signature verification, NSIS/MSI bundling, deep links, capability-based permissions for the WebView.
- One UI codebase for Windows, macOS and Linux later.

**Rejected.**
- **Electron:** proven, but each app ships its own Chromium and Node. Large installers and high idle memory contradict "small and quiet". The engine would need a native addon or a separate process anyway.
- **Qt (C++ or via bindings):** native performance and look, but QML/Widgets would not let us share components with the website, licensing (LGPL/commercial) adds friction, and the styling we want is easier on the web stack.
- **WinUI 3 / WPF:** best Windows integration, but Windows-only, which blocks the macOS/Linux roadmap and means a rewrite later.
- **Flutter:** good rendering, but Dart for the UI plus Rust for the engine via FFI, and a less native-feeling text/input stack on desktop.
- **egui / Slint / iced (pure Rust UI):** attractive for size, but typography, accessibility and animation polish are not yet where our design bar is.

**Consequences.** WebView2 is required on Windows (preinstalled on Windows 11 and current Windows 10; the installer bootstraps it if missing). UI behaviour must be tested on each platform's WebView later.

## ADR-004: Svelte 5, TypeScript and Vite for the UI

**Decision.** Svelte 5 (runes) with TypeScript, built with Vite. Shared components and design tokens in `packages/ui`.

**Why.**
- Compiles to small, fast JavaScript; little runtime. Matters for startup time in the app and for the website's near-zero-JS budget.
- Runes give fine-grained reactivity that suits a list of hundreds of downloads updating several times per second.
- Same components work in the Tauri app and as Astro islands on the website.

**Rejected.**
- **React:** biggest ecosystem, but more runtime, and fine-grained updates of large live lists need extra care (memoization, external stores).
- **SolidJS:** excellent performance and a close second; smaller ecosystem and less familiar to contributors than Svelte for this kind of UI.
- **Vue:** good option; Svelte's compile-time approach and smaller output won for our size goals.

## ADR-005: SQLite via rusqlite on a dedicated thread

**Context.** We need durable, crash-safe storage for downloads, segments, queues, categories and settings, with transactional checkpoints.

**Decision.** SQLite (bundled build) through **rusqlite**, owned by a single DB actor thread; migrations as embedded SQL files tracked with `PRAGMA user_version`.

**Why.**
- SQLite in WAL mode gives atomic, durable transactions and survives power loss without corruption, which our checkpoint invariant depends on ([ENGINE.md](ENGINE.md#checkpointing-and-crash-safe-resume)).
- rusqlite is synchronous and thin; a dedicated thread with a message channel is simple, avoids blocking Tokio workers, and keeps all writes serialized by construction.
- The database is inspectable with standard tools, which helps debugging and bug reports.

**Rejected.**
- **sqlx (SQLite):** async API and compile-time checked queries are nice, but its SQLite driver runs on a worker thread internally anyway, compile-time checking needs a database at build time (or offline metadata), and it brings more dependencies. Revisit if query complexity grows.
- **sled / redb / other embedded KV stores:** no SQL for queue and history queries; less mature crash-recovery story for our access pattern (sled in particular).
- **JSON files:** no atomic multi-record updates; easy to corrupt on crash.

## ADR-006: WXT for the browser extension

**Decision.** WXT, Manifest V3, TypeScript, Svelte for popup and options.

**Why.**
- One codebase producing Chrome, Edge and Firefox builds with per-browser manifest differences handled in config.
- Vite-based (same toolchain as the app), fast dev reload, good TypeScript support, framework-agnostic.

**Rejected.**
- **Plasmo:** capable but more opinionated and React-leaning.
- **Hand-rolled build:** possible, but cross-browser manifest generation, reload and packaging are exactly what WXT already does well.
- **Manifest V2:** being phased out by Chromium; not an option for new extensions.

## ADR-007: Native messaging for browser-to-app communication

**Decision.** The extension talks to the app only through native messaging, via a tiny `boltget-native-host.exe` that relays to the app's named pipe.

**Why.**
- Only extensions listed in the host manifest can connect. A localhost HTTP or WebSocket server, by contrast, can be reached by any web page the user visits (CSRF, DNS rebinding, port scanning), and needs its own authentication scheme to be safe.
- It is the mechanism browser vendors document and support for exactly this case.

**Rejected.**
- **Localhost HTTP/WebSocket server** (used by some download managers): larger attack surface, firewall prompts, port conflicts.
- **Custom protocol handler (`boltget://`):** good for "open in app" links (and may be added for that), but cannot carry cookies safely, has no reply channel and shows a browser prompt.

Details: [BROWSER-INTEGRATION.md](BROWSER-INTEGRATION.md#native-messaging).

## ADR-008: yt-dlp and ffmpeg as on-demand sidecars

**Decision.** Use yt-dlp and ffmpeg as external processes, downloaded on first use, verified and updated by BoltGet. The extension only detects media and hands the page URL to the app.

**Why.** yt-dlp's site coverage and update cadence cannot be matched by a reimplementation; keeping extraction out of the extension keeps it store-compliant; not bundling keeps the installer small and lets yt-dlp update independently.

**Rejected.**
- **Bundling the binaries:** larger installer, stale extractors between releases.
- **Rust reimplementations of site extractors:** perpetual maintenance race with much smaller coverage.
- **Embedding Python:** large, complex, and still needs yt-dlp updates.

Details: [VIDEO.md](VIDEO.md).

## ADR-009: One engine crate shared by app and CLI

**Decision.** `crates/engine` is a library. `crates/cli` and `apps/desktop/src-tauri` both depend on it.

**Why.** One implementation of the hard parts, tested once. The CLI doubles as the benchmark harness and the easiest way to reproduce engine bugs.

**Rejected.** A separate always-running daemon with the app and CLI as clients: more moving parts (service install, lifecycle, auth) for no benefit on a single-user desktop. The [single owner rule](ARCHITECTURE.md#single-owner-rule) gives the same safety.

## ADR-010: Astro and Starlight for the website and docs

**Decision.** `apps/web` is an Astro static site with Svelte islands for interactive pieces (the hero visualization); user documentation is built with Starlight in the same Astro project. Hosted on Cloudflare Pages or Vercel (decided before v1.0, see [WEBSITE.md](WEBSITE.md#hosting)).

**Why.** Static HTML by default with JavaScript only where an island needs it, which is how we hit the performance budget. Svelte islands let the site reuse `packages/ui` components. Starlight gives search, navigation and good typography for docs with little effort.

**Rejected.**
- **Next.js:** excellent for apps; for a mostly static marketing site and docs, it ships more client JavaScript by default and pulls us toward a React stack we do not use elsewhere.
- **SvelteKit:** a reasonable choice; Astro's islands model and Starlight make the content-heavy site simpler.
- **Docusaurus / VitePress:** good docs tools, but would split the site and docs into two stacks.

## ADR-011: pnpm workspaces and a Cargo workspace in one monorepo

**Decision.** A single repository, `boltget/boltget`, with a pnpm workspace (`apps/*`, `packages/*`) and a Cargo workspace (`crates/*`, `apps/desktop/src-tauri`).

**Why.** Engine, app, extension and site change together (message types, design tokens, docs). One repo means one PR can change a protocol and both sides of it, with one CI run.

**Rejected.**
- **Multiple repositories:** version skew between engine, app and extension; cross-repo PRs.
- **npm / Yarn:** pnpm's strict dependency layout and disk efficiency suit a workspace; Yarn would also work.
- **Nx / Turborepo from day one:** not needed at this size. Turborepo can be added later for task caching if CI times demand it.

## ADR-012: Tauri updater with GitHub Releases

**Decision.** The Tauri updater plugin, with update artifacts hosted on GitHub Releases and per-channel update manifests served from the website. Update packages are signed with the Tauri updater key and verified by the app before installation.

**Why.** Built into Tauri, signature verification is mandatory in its design, and GitHub Releases is free, reliable hosting for open-source binaries.

**Rejected.** Custom updater (unnecessary risk), Microsoft Store only (limits distribution; may be added as an additional channel later), winget only (no in-app updates).

Details: [RELEASE.md](RELEASE.md).

## ADR-013: Azure Trusted Signing for Windows code signing

**Decision.** Sign Windows binaries and installers with Azure Trusted Signing before v1.0.

**Why.** Unsigned download managers trigger SmartScreen warnings and antivirus suspicion, which is fatal for trust. Trusted Signing is low-cost (on the order of USD 10 per month at the time of writing for the basic tier), avoids handling a private key on a hardware token, and integrates with CI.

**Rejected.** Traditional OV/EV certificates (more expensive, hardware token logistics in CI); staying unsigned (unacceptable for 1.0).

**Risk.** Eligibility requirements for Trusted Signing have changed over time (by organization type and country). If BoltGet is not eligible, the fallback is SignPath's free code signing for open-source projects, or an OV certificate. See [RELEASE.md](RELEASE.md#code-signing).

## ADR-014: GPL-3.0-or-later

**Decision.** The entire repository is licensed GPL-3.0-or-later.

**Why.** The download-manager space is full of closed, ad-stuffed repackages of other people's work. Copyleft ensures any distributed fork stays open and that users of a modified BoltGet get the source too. "Or later" lets the project adopt future GPL versions without relicensing.

**Rejected.**
- **MIT / Apache-2.0:** maximally permissive, which here mostly benefits closed repackagers.
- **MPL-2.0:** file-level copyleft is too easy to work around for an application.
- **AGPL-3.0:** its network clause targets software offered as a service, which BoltGet is not; it adds friction without benefit.

**Consequences.** Dependencies must be GPL-3.0 compatible (MIT, Apache-2.0, BSD, ISC, MPL-2.0, Unlicense, Zlib are fine). CI will check licenses with `cargo-deny` and a JavaScript license checker. Contributions are accepted under the DCO (see [CONTRIBUTING.md](../CONTRIBUTING.md#developer-certificate-of-origin-dco)).

## ADR-015: English-only product with a central string catalog

**Decision.** The app, extension and website ship in English only. All user-facing strings live in a central catalog (keyed messages, for example `packages/ui/src/messages/en.json` for the UI and a similar file per app) and are referenced by key; no user-facing literals in components or in the engine.

**Why.** A single language keeps the scope tight for 1.0 and keeps copy consistent. A central catalog costs almost nothing now, keeps wording reviewable in one place, and keeps the door open: localization is a possible post-1.0, community-driven effort, not a goal.

**Rejected.** Adopting an i18n library and translation platform now (cost without users who need it); scattering literals (makes later translation a rewrite).

## ADR-016: Windows first

**Decision.** v1.0 targets Windows 10 and 11 (x64; ARM64 as soon as CI supports it). macOS and Linux follow after 1.0.

**Why.** IDM, the product we measure ourselves against, is Windows-only, so that is where its users are. Shipping one platform well beats three platforms badly. The stack (Rust, Tauri, WXT) is cross-platform, and platform-specific code is isolated from day one, so the later ports are planned work, not rewrites.
